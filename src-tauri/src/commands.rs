//! Tauri command layer (typed contract — the query boundary).
//!
//! Every command is `#[specta::specta]` with typed args/return/error; tauri-specta
//! generates the matching typed JS function. `tauri::State` args are injected by
//! the runtime and excluded from the JS signature. JS never sees SQL.
//!
//! The state holds `Arc`s so blocking work can be moved onto `spawn_blocking`
//! without borrowing the request-scoped `State` (which is not `'static`).

use std::sync::Arc;

use tauri::{Emitter, Manager, State};

use crate::collect::AlignReport;
use crate::config::{CloseBehavior, ConfigStore, Language, LightweightExpand, Skin};
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::library::{self, DeviceLibrarySummary, LibraryEntry, UploadItem};
use crate::model::{
    App, CommonConfigSnippet, DeviceInfo, LocalGroup, LogsQuery, ModelStatsRow, PricingEntry,
    Provider, RunMode, SessionFilter, SessionGroup, SessionGroupCounts, SessionMessage,
    SessionQuery, SessionRow, SyncedGroup, TrendBucket, TrendPoint, UsageFilter, UsageLogRow,
    UsageStats,
};
use crate::pricing;
use crate::provider::export_import::{ProviderImportMode, ProviderImportReport};
use crate::sessions;
use crate::sync::VerifyReport;

/// App-wide managed state: the Local Store + local config, wrapped
/// in `Arc` so blocking tasks can take owned clones.
pub struct AppState {
    pub store: Arc<Store>,
    pub config: Arc<ConfigStore>,
}

/// Snapshot of app/status info for the UI on startup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct AppInfo {
    pub device_id: String,
    pub display_name: String,
    pub mode: RunMode,
    pub repo_url: Option<String>,
    pub masked_token: Option<String>,
    pub github_user: Option<String>,
    pub claude_projects_dir: Option<String>,
    pub version: String,
}

// ---------------- App info / config ----------------

/// App status: device, mode (Standalone/Synced), paths, version.
#[tauri::command]
#[specta::specta]
pub fn get_app_info(state: State<'_, AppState>) -> AppResult<AppInfo> {
    let cfg = state.config.get();
    let claude_dir = crate::source_parser::default_projects_dir().map(|p| p.display().to_string());
    Ok(AppInfo {
        device_id: cfg.device_id.clone(),
        display_name: cfg.display_name.clone(),
        mode: cfg.mode(),
        repo_url: cfg.repo_url.clone(),
        masked_token: cfg.masked_token(),
        github_user: cfg.github_user.clone(),
        claude_projects_dir: claude_dir,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Configure the sync repo + PAT, upgrading Standalone → Synced, then
/// immediately run one sync round (pull peers + push self) so peer devices show
/// up and this device's existing data reaches the repo without a restart (the
/// startup sync only fires on next launch). Routed through `collect::sync_round`
/// — the same primitive the scheduler runs each push interval — but WITHOUT the
/// retry wrapping that `align` (the manual collect/sync buttons) applies: a
/// failure here is logged and left for the next startup sync to retry, not
/// retried in place. Best-effort: a sync failure doesn't undo the bind.
#[tauri::command]
#[specta::specta]
pub async fn set_sync_repo(
    state: State<'_, AppState>,
    repo_url: String,
    github_token: String,
) -> AppResult<RunMode> {
    let config = state.config.clone();
    let store = state.store.clone();
    let mode = tauri::async_runtime::spawn_blocking(move || -> AppResult<RunMode> {
        let cfg = config.update(|c| {
            c.repo_url = if repo_url.trim().is_empty() {
                None
            } else {
                Some(repo_url.trim().to_string())
            };
            c.github_token = if github_token.trim().is_empty() {
                None
            } else {
                Some(github_token.trim().to_string())
            };
        })?;
        if cfg.is_synced() {
            let outcome = crate::collect::sync_round(&store, &config);
            if outcome.imported > 0 {
                eprintln!(
                    "[cc-one] set_sync_repo imported {} row(s)",
                    outcome.imported
                );
            }
            if outcome.pushed {
                eprintln!("[cc-one] set_sync_repo pushed local changes");
            }
            for e in &outcome.errors {
                eprintln!("[cc-one] set_sync_repo sync error: {e}");
            }
        }
        Ok(cfg.mode())
    })
    .await
    .map_err(|e| AppError::Internal(format!("set_sync_repo task failed: {e}")))??;
    Ok(mode)
}

/// Unbind the repo, downgrading to Standalone. Clears the local
/// `.git` so a re-bind (often to a different repo) starts clean instead of
/// reusing the old remote/branch. Usage rows (DB) and `data/` are retained.
#[tauri::command]
#[specta::specta]
pub fn clear_sync_repo(state: State<'_, AppState>) -> AppResult<RunMode> {
    let cfg = state.config.update(|c| {
        c.repo_url = None;
        c.github_token = None;
    })?;
    let paths = state.config.paths();
    crate::sync::reset_local_git(&paths.repo);
    Ok(cfg.mode())
}

/// Probe a sync repo + PAT for reachability (「测试连接」). Pass explicit
/// values to validate BEFORE binding, or `None`/`None` to re-check the already-
/// configured repo. Pure ls-remote — never mutates config or the real sync repo.
/// Always returns `Ok(report)`; the probe's own outcome (auth ok / bad token /
/// not found) lives in `report.ok`, so the frontend never throws on a failed
/// probe (only a `spawn_blocking` join failure surfaces as an `AppError`).
#[tauri::command]
#[specta::specta]
pub async fn verify_sync_repo(
    state: State<'_, AppState>,
    repo_url: Option<String>,
    github_token: Option<String>,
) -> AppResult<VerifyReport> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<VerifyReport> {
        let cfg = config.get();
        let report = match (repo_url, github_token) {
            // Validate an as-yet-unbound pair straight from the Settings inputs.
            (Some(url), Some(tok)) => crate::sync::verify_remote(&url, &tok),
            // Re-check the configured repo: the raw PAT never crosses to JS, so
            // the masked_token the UI shows can't drive a re-probe — read the
            // real token server-side from config.
            (None, None) => match (cfg.repo_url.as_deref(), cfg.github_token.as_deref()) {
                (Some(url), Some(tok)) => crate::sync::verify_remote(url, tok),
                _ => crate::sync::verify_remote("", ""),
            },
            // One field present, the other absent: surface as an input error.
            _ => crate::sync::verify_remote("", ""),
        };
        Ok(report)
    })
    .await
    .map_err(|e| AppError::Internal(format!("verify task failed: {e}")))?
}

/// Rename *this* device (display name only — not a uniqueness key).
#[tauri::command]
#[specta::specta]
pub fn set_display_name(state: State<'_, AppState>, display_name: String) -> AppResult<()> {
    crate::devices::rename_self(&state.store, &state.config, &display_name)
}

/// Set a friendly name for *another* device seen in the repo (map).
#[tauri::command]
#[specta::specta]
pub fn set_device_display_name(
    state: State<'_, AppState>,
    device_id: String,
    display_name: String,
) -> AppResult<()> {
    crate::devices::rename_peer(&state.store, &state.config, &device_id, &display_name)
}

/// Locally forget a peer device: drop its registry row + all its local usage
/// data (records, turn durations) + its local artifact dir, and clear any local
/// alias. `library_action` decides the fate of the peer's library subtree
/// (`repo/library/<id>/`): migrated into this device's library or deleted.
/// Nothing is pushed to Git — a peer still in the repo reappears on the next
/// sync (registry + data artifacts are re-imported). This device (`is_self`) is
/// not forgettable; rename it instead.
#[tauri::command]
#[specta::specta]
pub fn forget_device(
    state: State<'_, AppState>,
    device_id: String,
    library_action: crate::library::LibraryForgetAction,
) -> AppResult<()> {
    let cfg = state.config.get();
    if crate::devices::is_self(&cfg, &device_id) {
        return Err(AppError::Config(
            "this device cannot be removed (rename it instead)".into(),
        ));
    }
    // Capture the peer's alias BEFORE the registry row + alias map are dropped —
    // the migrate target folder is named after it (`from-<name>`). The full
    // five-step cleanup (DB row, alias, data dir, name file, library subtree)
    // is owned by `devices::forget_device`.
    let peer_name = cfg
        .device_names
        .get(&device_id)
        .cloned()
        .unwrap_or_default();
    crate::devices::forget_device(
        &state.store,
        &state.config,
        &state.config.paths(),
        &device_id,
        library_action,
        &peer_name,
    )
}

// ---------------- Collect / ingest ----------------
// The ingest path (`collect_into`) and the manual orchestrators (`align`,
// `sync_round`) live in `collect`; the items here are the typed Tauri commands
// that drive them.

/// Manual「采集 / 同步」: collect now, then (Synced only) pull + push with a
/// bounded retry. The dashboard button's single action — Standalone ⇒ collect;
/// Synced ⇒ collect + sync. The run mode decides what it means, not the UI.
/// Heavy disk/git work → offloaded to a thread.
#[tauri::command]
#[specta::specta]
pub async fn collect_now(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<AlignReport> {
    let store = state.store.clone();
    let config = state.config.clone();
    let report =
        tauri::async_runtime::spawn_blocking(move || crate::collect::align(&store, &config))
            .await
            .map_err(|e| AppError::Internal(format!("collect task failed: {e}")))?;
    // Notify the UI that usage data changed (event-driven refresh).
    let _ = app_handle.emit("usage_changed", ());
    Ok(report)
}

/// Manual「立即同步」: the Settings entry — same `align` as the dashboard button
/// (collect + sync). Kept as a distinct command so the Settings card has its
/// own trigger next to the repo binding, but the work is identical. Standalone
/// ⇒ collect only (sync degrades to a local refresh).
#[tauri::command]
#[specta::specta]
pub async fn sync_now(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<AlignReport> {
    let store = state.store.clone();
    let config = state.config.clone();
    let report =
        tauri::async_runtime::spawn_blocking(move || crate::collect::align(&store, &config))
            .await
            .map_err(|e| AppError::Internal(format!("sync task failed: {e}")))?;
    let _ = app_handle.emit("usage_changed", ());
    Ok(report)
}

/// Rebill zero-cost rows whose model now has a price (top-up).
#[tauri::command]
#[specta::specta]
pub fn rebill_zero_cost(state: State<'_, AppState>) -> AppResult<u32> {
    let book = state.store.load_pricing_book()?;
    Ok(state.store.rebill_zero_cost(&book)? as u32)
}

// ---------------- Dashboard reads ----------------

#[tauri::command]
#[specta::specta]
pub fn query_usage_stats(state: State<'_, AppState>, filter: UsageFilter) -> AppResult<UsageStats> {
    state.store.query_stats(&filter)
}

#[tauri::command]
#[specta::specta]
pub fn query_usage_trend(
    state: State<'_, AppState>,
    filter: UsageFilter,
    bucket: TrendBucket,
) -> AppResult<Vec<TrendPoint>> {
    state.store.query_trend(&filter, bucket)
}

#[tauri::command]
#[specta::specta]
pub fn query_usage_logs(
    state: State<'_, AppState>,
    query: LogsQuery,
) -> AppResult<Vec<UsageLogRow>> {
    state.store.query_logs(&query)
}

#[tauri::command]
#[specta::specta]
pub fn count_usage_logs(state: State<'_, AppState>, filter: UsageFilter) -> AppResult<u32> {
    state.store.count_logs(&filter)
}

#[tauri::command]
#[specta::specta]
pub fn query_models(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<ModelStatsRow>> {
    state.store.query_models(&filter)
}

#[tauri::command]
#[specta::specta]
pub fn query_distinct_sources(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    state.store.query_distinct("source")
}

#[tauri::command]
#[specta::specta]
pub fn query_distinct_models(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    state.store.query_distinct("model")
}

#[tauri::command]
#[specta::specta]
pub fn list_devices(state: State<'_, AppState>) -> AppResult<Vec<DeviceInfo>> {
    let mut devices = state.store.list_devices()?;
    let cfg = state.config.get();
    crate::devices::apply_aliases(&mut devices, &cfg);
    // NOTE: duplicate display names are no longer disambiguated with an id
    // prefix — the picker shows the raw name and truncates with an ellipsis if
    // it overflows. Users tell peers apart by renaming them in Settings.
    Ok(devices)
}

// ---------------- Pricing ----------------

#[tauri::command]
#[specta::specta]
pub fn list_pricing(state: State<'_, AppState>) -> AppResult<Vec<PricingEntry>> {
    state.store.list_pricing()
}

/// Add or update a pricing entry from the UI (user edits ⇒ `is_builtin=false`).
#[tauri::command]
#[specta::specta]
pub fn save_pricing_entry(
    state: State<'_, AppState>,
    entry: PricingEntry,
    is_builtin: Option<bool>,
) -> AppResult<()> {
    let mut entry = entry;
    entry.is_builtin = is_builtin.unwrap_or(false);
    state.store.upsert_pricing(&entry)
}

#[tauri::command]
#[specta::specta]
pub fn delete_pricing_entry(state: State<'_, AppState>, model_key: String) -> AppResult<()> {
    state.store.delete_pricing(&model_key)
}

/// Re-load pricing from the local `pricing.json` into the DB. Pricing is
/// per-device local (never synced); this is an import surface, not a sync path.
#[tauri::command]
#[specta::specta]
pub fn reload_pricing_from_file(state: State<'_, AppState>) -> AppResult<u32> {
    let path = state.config.paths().pricing_json();
    if !path.exists() {
        return Err(AppError::Pricing(format!(
            "pricing.json not found at {}",
            path.display()
        )));
    }
    state.store.reload_pricing_from_path(&path)
}

/// Persist current DB pricing to the local `pricing.json` (never synced).
#[tauri::command]
#[specta::specta]
pub fn save_pricing_to_file(state: State<'_, AppState>) -> AppResult<()> {
    let entries = state.store.load_pricing_models()?;
    let doc = pricing::write_pricing_doc(&entries)?;
    let path = state.config.paths().pricing_json();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, doc)?;
    Ok(())
}

/// Fetch LiteLLM upstream pricing and merge into the DB (seed).
/// Network → async + offloaded. Best-effort: returns count merged (0 offline).
#[tauri::command]
#[specta::specta]
pub async fn fetch_litellm_pricing(state: State<'_, AppState>) -> AppResult<u32> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<u32> {
        let entries = crate::pricing::fetch_litellm()?;
        let mut merged = 0u32;
        for e in &entries {
            store.upsert_pricing(&e.to_entry())?;
            merged += 1;
        }
        Ok(merged)
    })
    .await
    .map_err(|e| AppError::Pricing(format!("litellm task failed: {e}")))?
}

// ---------------- Preferences (tray + background) ----------------

/// User-tunable preferences surfaced in the Settings「通用」card.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct Preferences {
    pub close_behavior: CloseBehavior,
    pub collect_interval_secs: u32,
    pub push_interval_secs: u32,
    pub language: Language,
    pub lightweight_expand: LightweightExpand,
    pub skin: Skin,
}

fn to_preferences(cfg: &crate::config::ConfigData) -> Preferences {
    Preferences {
        close_behavior: cfg.close_behavior,
        collect_interval_secs: cfg.collect_interval_secs,
        push_interval_secs: cfg.push_interval_secs,
        language: cfg.language,
        lightweight_expand: cfg.lightweight_expand,
        skin: cfg.skin,
    }
}

/// Read the current preferences for the Settings card.
#[tauri::command]
#[specta::specta]
pub fn get_preferences(state: State<'_, AppState>) -> AppResult<Preferences> {
    Ok(to_preferences(&state.config.get()))
}

/// Persist the window-close behavior.
#[tauri::command]
#[specta::specta]
pub fn set_close_behavior(
    state: State<'_, AppState>,
    close_behavior: CloseBehavior,
) -> AppResult<Preferences> {
    let cfg = state.config.update(|c| c.close_behavior = close_behavior)?;
    Ok(to_preferences(&cfg))
}

/// Persist the background-collect interval (seconds, clamped to [10, 3600];
///). Pure-local cadence — does not touch the network.
#[tauri::command]
#[specta::specta]
pub fn set_collect_interval(state: State<'_, AppState>, seconds: u32) -> AppResult<Preferences> {
    let clamped = seconds.clamp(5, 3600);
    let cfg = state.config.update(|c| c.collect_interval_secs = clamped)?;
    Ok(to_preferences(&cfg))
}

/// Persist the push-to-sync interval (seconds, clamped to [60, 7200]; Synced
/// only). Decoupled from collect so the Git history grows at this
/// rate, not the (shorter) collect rate.
#[tauri::command]
#[specta::specta]
pub fn set_push_interval(state: State<'_, AppState>, seconds: u32) -> AppResult<Preferences> {
    let clamped = seconds.clamp(60, 7200);
    let cfg = state.config.update(|c| c.push_interval_secs = clamped)?;
    Ok(to_preferences(&cfg))
}

/// Persist the display language and rebuild the tray menu so the
/// "Quit" item follows the new language immediately. The tray item is the only
/// user-facing Rust string; all other UI text is frontend i18n driven by this
/// same preference.
#[tauri::command]
#[specta::specta]
pub fn set_language(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    language: Language,
) -> AppResult<Preferences> {
    let cfg = state.config.update(|c| c.language = language)?;
    if let Some(tray) = app_handle.tray_by_id("main") {
        if let Ok(menu) = crate::tray_menu_for(&app_handle, language) {
            let _ = tray.set_menu(Some(menu));
        }
    }
    Ok(to_preferences(&cfg))
}

/// Swap the tray AND taskbar/Alt-Tab icon between the dark-badge and
/// light-badge PNG so both track the resolved UI theme. Stateless — theme
/// truth lives in next-themes (localStorage); Rust never persists or reads it
/// back. The frontend pushes the *resolved* dark/light value, not the user's
/// "system" choice, because `system` only resolves on the JS side (via the OS
/// theme) and the icon needs a concrete asset now. The taskbar icon is set
/// live via `set_icon`; the Start-menu / exe icon is a packaged resource that
/// can't change at runtime.
#[tauri::command]
#[specta::specta]
pub fn set_tray_theme(app_handle: tauri::AppHandle, dark: bool) -> AppResult<()> {
    if let Some(tray) = app_handle.tray_by_id("main") {
        let bytes: &[u8] = if dark {
            include_bytes!("../icons/tray-dark.png")
        } else {
            include_bytes!("../icons/tray-light.png")
        };
        if let Ok(icon) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(icon.clone()));
            if let Some(window) = app_handle.get_webview_window("main") {
                // window.set_icon swaps the taskbar + Alt-Tab icon live.
                let _ = window.set_icon(icon);
            }
        }
    }
    Ok(())
}

/// Persist the lightweight half-icon expand trigger. Pure frontend
/// behavior; Rust doesn't read it back, but it rides ConfigData for unity.
#[tauri::command]
#[specta::specta]
pub fn set_lightweight_expand(
    state: State<'_, AppState>,
    lightweight_expand: LightweightExpand,
) -> AppResult<Preferences> {
    let cfg = state
        .config
        .update(|c| c.lightweight_expand = lightweight_expand)?;
    Ok(to_preferences(&cfg))
}

/// Persist the color skin (multi-skin theming). Pure frontend effect — Rust
/// never reads it back; it rides ConfigData for unity with the other prefs.
#[tauri::command]
#[specta::specta]
pub fn set_skin(state: State<'_, AppState>, skin: Skin) -> AppResult<Preferences> {
    let cfg = state.config.update(|c| c.skin = skin)?;
    Ok(to_preferences(&cfg))
}

/// Resolve the one-time close dialog. `remember` pins `choice` as
/// the persisted behavior; the chosen action is then executed immediately.
/// `Minimize`/`Ask` hide the window (scheduler keeps running); `Quit` exits.
#[tauri::command]
#[specta::specta]
pub fn confirm_close(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    choice: CloseBehavior,
    remember: bool,
) -> AppResult<()> {
    if remember {
        let _ = state.config.update(|c| c.close_behavior = choice);
    }
    match choice {
        CloseBehavior::Quit => app_handle.exit(0),
        CloseBehavior::Minimize | CloseBehavior::Ask => {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.hide();
            }
        }
    }
    Ok(())
}

// ---------------- Sessions ----------------

/// Emit `sessions_changed` so the frontend's session queries invalidate.
fn emit_sessions_changed(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit("sessions_changed", ());
}

#[tauri::command]
#[specta::specta]
pub fn query_sessions_cmd(
    state: State<'_, AppState>,
    query: SessionQuery,
) -> AppResult<Vec<SessionRow>> {
    // Paged — the UI renders one page instead of loading every session.
    state.store.query_sessions_page(&query)
}

/// Sidebar + paginator counts for one grouping track under a filter — the
/// "All" row total, the group-row buckets, and (derived client-side) the
/// ungrouped count. Paging-independent: describes the whole filtered set.
#[tauri::command]
#[specta::specta]
pub fn count_sessions_cmd(
    state: State<'_, AppState>,
    filter: Option<SessionFilter>,
    track: String,
) -> AppResult<SessionGroupCounts> {
    state.store.count_sessions(filter.as_ref(), &track)
}

#[tauri::command]
#[specta::specta]
pub fn get_session_transcript_cmd(
    state: State<'_, AppState>,
    id: String,
    device_id: String,
) -> AppResult<Vec<SessionMessage>> {
    // The transcript lives in the db (`session_messages`) for every session —
    // favorited or not — so this read no longer depends on the favorites-only
    // jsonl snapshot. `device_id` is the own device; its rows win on uuid
    // conflict (it is the source of truth for a session it collected), then
    // peers' pulled-in rows fill the gaps.
    state.store.query_session_transcript(&id, &device_id)
}

#[tauri::command]
#[specta::specta]
pub fn set_session_favorited_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    device_id: String,
    favorited: bool,
) -> AppResult<()> {
    state
        .store
        .set_session_favorited(&device_id, &id, favorited)?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_session_custom_title_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    device_id: String,
    title: Option<String>,
) -> AppResult<()> {
    state
        .store
        .set_session_custom_title(&device_id, &id, title.as_deref())?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_session_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    device_id: String,
    group_id: Option<String>,
) -> AppResult<()> {
    state
        .store
        .set_session_local_group(&device_id, &id, group_id.as_deref())?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_session_synced_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    device_id: String,
    group_id: Option<String>,
) -> AppResult<()> {
    state
        .store
        .set_session_synced_group(&device_id, &id, group_id.as_deref())?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

// ---- local groups ----

#[tauri::command]
#[specta::specta]
pub fn list_local_groups_cmd(state: State<'_, AppState>) -> AppResult<Vec<LocalGroup>> {
    state.store.list_local_groups()
}

#[tauri::command]
#[specta::specta]
pub fn create_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
) -> AppResult<LocalGroup> {
    let id = sessions::generate_local_group_id();
    let created_at = crate::time::now_iso();
    let group = state
        .store
        .create_local_group(&id, name.trim(), &created_at)?;
    emit_sessions_changed(&app_handle);
    Ok(group)
}

#[tauri::command]
#[specta::specta]
pub fn rename_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    name: String,
) -> AppResult<()> {
    state.store.rename_local_group(&id, name.trim())?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn delete_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> AppResult<()> {
    state.store.delete_local_group(&id)?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn reorder_local_groups_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    state.store.reorder_local_groups(&ordered_ids)?;
    emit_sessions_changed(&app_handle);
    Ok(())
}

// ---- synced groups ----

#[tauri::command]
#[specta::specta]
pub fn list_synced_groups_cmd(state: State<'_, AppState>) -> AppResult<Vec<SyncedGroup>> {
    Ok(sessions::read_all_synced_groups(&state.config.paths()))
}

#[tauri::command]
#[specta::specta]
pub async fn create_synced_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
) -> AppResult<SyncedGroup> {
    let config = state.config.clone();
    let group = tauri::async_runtime::spawn_blocking(move || -> AppResult<SyncedGroup> {
        let cfg = config.get();
        let paths = config.paths();
        sessions::create_synced_group_owned(&paths, &cfg, &name)
    })
    .await
    .map_err(|e| AppError::Internal(format!("create_synced_group task failed: {e}")))??;
    emit_sessions_changed(&app_handle);
    Ok(group)
}

#[tauri::command]
#[specta::specta]
pub async fn rename_synced_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    name: String,
) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let cfg = config.get();
        let paths = config.paths();
        sessions::rename_synced_group_owned(&paths, &cfg, &id, &name)
    })
    .await
    .map_err(|e| AppError::Internal(format!("rename_synced_group task failed: {e}")))??;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_synced_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let cfg = config.get();
        let paths = config.paths();
        sessions::delete_synced_group_owned(&paths, &cfg, &id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("delete_synced_group task failed: {e}")))??;
    emit_sessions_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_synced_groups_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let cfg = config.get();
        let paths = config.paths();
        sessions::reorder_synced_groups_owned(&paths, &cfg, &ordered_ids)
    })
    .await
    .map_err(|e| AppError::Internal(format!("reorder_synced_group task failed: {e}")))??;
    emit_sessions_changed(&app_handle);
    Ok(())
}

/// Unified groups list (local + synced) for one-shot UI fetch.
#[tauri::command]
#[specta::specta]
pub fn list_groups_cmd(state: State<'_, AppState>) -> AppResult<Vec<SessionGroup>> {
    sessions::list_groups_dto(&state.store, &state.config.paths())
}

// ---------------- Providers (供应商) ----------------

/// Emit `providers_changed` so the frontend's provider queries invalidate.
fn emit_providers_changed(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit("providers_changed", ());
}

/// 列出一个应用池的供应商（app 必填——前端传当前分段 tab，后端按池过滤）。
#[tauri::command]
#[specta::specta]
pub fn list_providers_cmd(state: State<'_, AppState>, app: App) -> AppResult<Vec<Provider>> {
    state.store.list_providers_for(app)
}

/// Upsert a provider (empty id = create, non-empty = edit). Returns the
/// persisted row so the caller learns the assigned id / sort position without
/// a second read. The provider carries its `app` (the pool it belongs to).
#[tauri::command]
#[specta::specta]
pub fn save_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    provider: Provider,
) -> AppResult<Provider> {
    let saved = state.store.save_provider(provider)?;
    emit_providers_changed(&app_handle);
    Ok(saved)
}

#[tauri::command]
#[specta::specta]
pub fn delete_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<()> {
    state.store.delete_provider(app, &id)?;
    emit_providers_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn reorder_providers_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    state.store.reorder_providers(app, &ordered_ids)?;
    emit_providers_changed(&app_handle);
    Ok(())
}

/// 切换供应商（核心动作）：按 (app, id) 查 provider → 按应用分派写盘 →
/// 记该应用的激活状态。写盘分派 `write_live(app, provider)`：claude 走 JSON
/// 受控合并进 `~/.claude/settings.json`（合并前先叠该应用的通用片段、拦截
/// 未物化模板变量），codex 走 TOML 受控合并 + auth.json，gemini 走 env 整块
/// 替换 + settings.json 受控合并。各分支语义一致：只替换受控字段、非受控
/// 字段（hooks / MCP / permissions / model / mcp_servers 等）从 live 原地
/// 保留，不整文件覆盖、不做 Backfill。「保存」只写 DB（save_provider_cmd），
/// 本命令才真正写盘。
#[tauri::command]
#[specta::specta]
pub async fn switch_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<Provider> {
    let store = state.store.clone();
    let config = state.config.clone();
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<Provider> {
        let provider = store
            .get_provider(app, &id)?
            .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
        // 写盘分派（provider::live::write_live）：claude 侧先叠该应用的通用
        // 片段（片段按 provider 归属的应用读取——claude 池读 claude 片段，
        // 存量迁移后即原全局片段，行为不变）并拦截未物化模板变量，再受控
        // 写盘；codex/gemini 直接写 provider 配置（其片段支持属后续批次）。
        // 各分支写盘语义一致——只替换受控字段、非受控字段从 live 原地保留。
        let write_provider = if app == App::Claude {
            let cfg = config.get();
            let snippet = cfg.snippet_for(provider.app);
            let settings_config = crate::provider::snippet::apply_snippet(
                &provider.settings_config,
                &snippet.content,
                snippet.enabled,
            )?;
            // 字面量 `${VAR}` 写进 live 文件等于写一份废配置。
            crate::provider::live::validate_no_unfilled_template_vars(&settings_config)?;
            Provider {
                settings_config,
                ..provider.clone()
            }
        } else {
            provider.clone()
        };
        crate::provider::live::write_live(app, &write_provider)?;
        config.update(|c| c.set_active_provider(app, &id))?;
        Ok(provider)
    })
    .await
    .map_err(|e| AppError::Internal(format!("switch_provider task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 当前激活的完整 provider（前端「当前使用」光卡用，按应用查询）。未激活、
/// 或激活的 provider 已被删除 → `None`。
#[tauri::command]
#[specta::specta]
pub fn get_active_provider_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<Option<Provider>> {
    let id = match state.config.get().active_provider_id_for(app) {
        Some(id) => id,
        None => return Ok(None),
    };
    state.store.get_provider(app, &id)
}

/// 读某应用的通用配置片段（内容 + 启用开关）。按应用各存一份（claude /
/// codex / gemini），存本机 config.json。缺省键按应用回退默认（claude 为
/// 隐藏署名片段，其余为空片段）。
#[tauri::command]
#[specta::specta]
pub fn get_common_config_snippet_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<CommonConfigSnippet> {
    Ok(state.config.get().snippet_for(app))
}

/// 保存某应用的通用配置片段。内容必须是合法 JSON 对象（空串视为空片段）；
/// 非法 JSON 拒绝保存（`AppError::Config`）。写盘合并只认受控字段，非受控
/// 键在写盘时被忽略。
#[tauri::command]
#[specta::specta]
pub fn set_common_config_snippet_cmd(
    state: State<'_, AppState>,
    app: App,
    json: String,
    enabled: bool,
) -> AppResult<CommonConfigSnippet> {
    crate::provider::snippet::validate_snippet(&json)?;
    let snippet = CommonConfigSnippet {
        enabled,
        content: json,
    };
    state.config.update(|c| c.set_snippet(app, snippet))?;
    Ok(state.config.get().snippet_for(app))
}

/// 导出全部供应商为 JSON 文档，写入 `target_path`（前端 save 对话框选的位置）。
/// `include_keys=false` 时剔除 settingsConfig env 里的密钥键。换设备迁移 /
/// 留档用，不经过 git 同步。返回文档里的 provider 数量。
#[tauri::command]
#[specta::specta]
pub fn export_providers_cmd(
    state: State<'_, AppState>,
    include_keys: bool,
    target_path: String,
) -> AppResult<u32> {
    let providers = state.store.list_providers()?;
    let doc = crate::provider::export_import::export_document(
        &providers,
        include_keys,
        &crate::time::now_iso(),
    )?;
    std::fs::write(&target_path, doc)?;
    Ok(providers.len() as u32)
}

/// 从 JSON 文档导入供应商（合并 / 覆盖模式）。`source_path` 是前端 open
/// 对话框选的文件。只写本机 DB（`save_provider`），不触发 providers.json
/// 同步写——导入的 key 只进本机库。返回应用 / 跳过计数。
#[tauri::command]
#[specta::specta]
pub fn import_providers_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    source_path: String,
    mode: ProviderImportMode,
) -> AppResult<ProviderImportReport> {
    let json = std::fs::read_to_string(&source_path)?;
    let report = crate::provider::export_import::apply_import(&state.store, &json, mode)?;
    emit_providers_changed(&app_handle);
    Ok(report)
}

/// 获取供应商的可用模型列表。`app` 决定端点格式：claude / codex 走 OpenAI
/// 兼容 `GET /v1/models`（当前实现），gemini 的 Google 原生路径由后续批次
/// 实现——本票只把 app 维度接进签名，gemini 分支显式报错而不是静默走错
/// 端点。WebView fetch 撞 CORS，所以请求由后端发（ureq）。`models_url`
/// 非空时精确覆写候选列表（只试这一个）；否则对 baseURL 构造候选 URL
/// （版本段识别 + 兼容子路径剥离，见 `provider::model_fetch::
/// candidate_models_urls`），按序尝试首个成功。错误串带稳定前缀标签
/// （AUTH_FAILED / ENDPOINT_CLOSED / TIMEOUT / BAD_FORMAT / NETWORK），
/// 前端按标签分桶提示。
#[tauri::command]
#[specta::specta]
pub async fn fetch_models_cmd(
    app: App,
    base_url: String,
    api_key: String,
    models_url: Option<String>,
) -> AppResult<Vec<String>> {
    if app == App::Gemini {
        return Err(AppError::Config(
            "gemini model fetch is not implemented yet".into(),
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        crate::provider::model_fetch::fetch_models(&base_url, &api_key, models_url.as_deref())
    })
    .await
    .map_err(|e| AppError::Internal(format!("fetch_models task failed: {e}")))?
}

// ---------------- Library ----------------

#[tauri::command]
#[specta::specta]
pub async fn scan_library(
    state: State<'_, AppState>,
    device_scope: String,
    subpath: String,
) -> AppResult<Vec<LibraryEntry>> {
    let config = state.config.clone();
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        library::scan(&store, &config, &device_scope, &subpath)
    })
    .await
    .map_err(|e| AppError::Internal(format!("library scan task failed: {e}")))?
}

#[tauri::command]
#[specta::specta]
pub async fn upload_to_library(
    state: State<'_, AppState>,
    items: Vec<UploadItem>,
    subpath: String,
) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let cfg = config.get();
        let paths = config.paths();
        library::upload(&paths, &cfg, &items, &subpath)?;
        library::commit_push_library(&paths, &cfg);
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("library upload task failed: {e}")))?
}

#[tauri::command]
#[specta::specta]
pub async fn export_from_library(
    state: State<'_, AppState>,
    rel_path: String,
    target_dir: String,
) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let paths = config.paths();
        library::export_entry(&paths, &rel_path, &target_dir)
    })
    .await
    .map_err(|e| AppError::Internal(format!("library export task failed: {e}")))?
}

#[tauri::command]
#[specta::specta]
pub async fn delete_from_library(state: State<'_, AppState>, rel_path: String) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let cfg = config.get();
        let paths = config.paths();
        library::delete_entry(&paths, &rel_path)?;
        library::commit_push_library(&paths, &cfg);
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("library delete task failed: {e}")))?
}

#[tauri::command]
#[specta::specta]
pub async fn rename_in_library(
    state: State<'_, AppState>,
    rel_path: String,
    new_name: String,
) -> AppResult<()> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        let cfg = config.get();
        let paths = config.paths();
        library::rename_entry(&paths, &rel_path, &new_name)?;
        library::commit_push_library(&paths, &cfg);
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("library rename task failed: {e}")))?
}

/// Read a library entry as text for the themed preview (see
/// [`library::read_text_entry`]).
#[tauri::command]
#[specta::specta]
pub async fn read_library_text(
    state: State<'_, AppState>,
    rel_path: String,
) -> AppResult<Option<String>> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let paths = config.paths();
        library::read_text_entry(&paths, &rel_path)
    })
    .await
    .map_err(|e| AppError::Internal(format!("library text read task failed: {e}")))?
}

/// File/folder counts for one device's library subtree — used by the
/// forget-device dialog to show what would be migrated or deleted.
#[tauri::command]
#[specta::specta]
pub async fn library_device_summary(
    state: State<'_, AppState>,
    device_id: String,
) -> AppResult<DeviceLibrarySummary> {
    let config = state.config.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let paths = config.paths();
        Ok(library::count_subtree(&paths.library.join(&device_id)))
    })
    .await
    .map_err(|e| AppError::Internal(format!("library summary task failed: {e}")))?
}
