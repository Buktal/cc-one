//! Tauri command layer (typed contract — the query boundary).
//!
//! Every command is `#[specta::specta]` with typed args/return/error; tauri-specta
//! generates the matching typed JS function. `tauri::State` args are injected by
//! the runtime and excluded from the JS signature. JS never sees SQL.
//!
//! The state holds `Arc`s so blocking work can be moved onto `spawn_blocking`
//! without borrowing the request-scoped `State` (which is not `'static`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
use tauri::{Emitter, Manager, State};

use crate::collect::AlignReport;
use crate::config::{CloseBehavior, ConfigStore, Language, LightweightExpand, Skin};
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::library::{self, DeviceLibrarySummary, LibraryEntry, UploadItem};
use crate::model::{
    App, CommonConfigSnippet, DeviceInfo, LocalGroup, LogsQuery, ModelStatsRow, PricingEntry,
    Provider, ProviderCategory, RunMode, SessionFilter, SessionGroup, SessionGroupCounts,
    SessionMessage, SessionQuery, SessionRow, SyncedGroup, TrendBucket, TrendPoint, UsageFilter,
    UsageLogRow, UsageStats,
};
use crate::pricing;
use crate::provider::export_import::{self, ProviderImportMode, ProviderImportReport};
use crate::provider::{
    import_ccswitch, import_live, live, live_codex, live_gemini, live_grok, live_opencode,
};
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
pub fn query_distinct_sources(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<String>> {
    state.store.query_distinct("source", &filter)
}

#[tauri::command]
#[specta::specta]
pub fn query_distinct_models(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<String>> {
    state.store.query_distinct("model", &filter)
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
    pub lightweight_auto_tuck_secs: u32,
    pub skin: Skin,
}

fn to_preferences(cfg: &crate::config::ConfigData) -> Preferences {
    Preferences {
        close_behavior: cfg.close_behavior,
        collect_interval_secs: cfg.collect_interval_secs,
        push_interval_secs: cfg.push_interval_secs,
        language: cfg.language,
        lightweight_expand: cfg.lightweight_expand,
        lightweight_auto_tuck_secs: cfg.lightweight_auto_tuck_secs,
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

/// Persist the auto-tuck delay (seconds; `0` = off) before an invisible full
/// window morphs into the mini bar. Pure frontend behavior; Rust stores it for
/// unity with the other Settings prefs.
#[tauri::command]
#[specta::specta]
pub fn set_lightweight_auto_tuck(state: State<'_, AppState>, secs: u32) -> AppResult<Preferences> {
    let cfg = state
        .config
        .update(|c| c.lightweight_auto_tuck_secs = secs)?;
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
pub async fn delete_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<()> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        // 附加模式：provider 若已写进 live，先从 live 移除再删 DB，避免配置文件
        // 残留 orphan 条目；单激活直接删 DB（其 live 由切换覆盖，无残留概念）。
        // liveManaged=true 才需移除（已移除的 provider liveManaged=false，live 里
        // 已没它，跳过免得无谓读写文件）。
        if app.is_additive_mode() {
            if let Some(provider) = store.get_provider(app, &id)? {
                if live_opencode::meta_live_managed(&provider.meta) == Some(true) {
                    if let Some(key) = live_opencode::meta_live_key(&provider.meta) {
                        let path = live_opencode::opencode_config_path()?;
                        live_opencode::remove_opencode_provider(&path, &key)?;
                    }
                }
            }
        }
        store.delete_provider(app, &id)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("delete_provider task failed: {e}")))??;
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
/// 记该应用的激活状态。写盘分派 `write_live(app, provider, snippet)`：claude
/// 走 JSON 受控合并进 `~/.claude/settings.json`，codex 走 TOML 受控合并 +
/// auth.json，gemini 走 env 整块替换 + settings.json 受控合并。各分支语义
/// 一致：只替换受控字段、非受控字段（hooks / MCP / permissions / model /
/// mcp_servers 等）从 live 原地保留，不整文件覆盖、不做 Backfill。
///
/// 通用片段按应用分派合并层（ADR-0010）：claude/gemini 在 settings_config 层
/// 并入（合并前先叠片段；claude 另拦截未物化模板变量——gemini 的 `.env` 由
/// dotenv 展开 `${VAR}` 是合法引用，不拦）；codex/grok 在写盘层补缺失（片段
/// 随 `snippet` 传给 `write_live`，受控合并后补进 live 文件——否则被白名单
/// 滤掉→零效果）。「保存」只写 DB（save_provider_cmd），本命令才真正写盘。
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
        // 附加模式（OpenCode）：ensure-in-live——写进 opencode.json + 设
        // liveManaged=true，**不取消其它 provider、不碰 active_providers**（附加
        // 模式无唯一激活）。返回更新了 meta 的 provider。
        if app.is_additive_mode() {
            return ensure_opencode_in_live(&store, provider);
        }
        // 单激活：按应用分派片段合并层（ADR-0010）——claude/gemini = settings_config
        // 层（片段先并入供应商配置、再随受控写盘落地）；codex/grok = 写盘层（受控
        // 合并之后、片段补缺失进 live 文件——否则被写盘白名单滤掉→片段零效果）。
        // 片段按 provider 归属的应用读取（claude 池读 claude 片段，存量迁移后即原
        // 全局片段，行为不变）。snippet_for 返回 owned，读 guard 随语句结束释放。
        let snippet_record = config.get().snippet_for(app);
        let write_provider = match app {
            // settings_config 层片段（claude/gemini）：片段先并入供应商配置，再随
            // 受控写盘落地。
            App::Claude | App::Gemini => {
                let settings_config = crate::provider::snippet::apply_snippet(
                    &provider.settings_config,
                    &snippet_record.content,
                    snippet_record.enabled,
                )?;
                // claude 的 settings.json 是字面量 JSON：${VAR} 占位符会原样写进
                // live = 废配置，切换前拦下。gemini 的 .env 由 dotenv 展开 ${VAR}
                // （合法引用，gemini 预设也不用模板变量），不拦。
                if app == App::Claude {
                    crate::provider::live::validate_no_unfilled_template_vars(&settings_config)?;
                }
                Provider {
                    settings_config,
                    ..provider.clone()
                }
            }
            // codex/grok 走写盘层（片段随 write_snippet 传给 write_live，受控合并
            // 后补缺失进 live 文件）。
            _ => provider.clone(),
        };
        // 写盘层片段（codex/grok）：启用 → 片段内容，否则空串（switch_*_live
        // 空串即无操作）。claude/gemini 一律空串（其片段已在 settings_config 层
        // 处理）。
        let write_snippet = match app {
            App::Codex | App::Grok if snippet_record.enabled => snippet_record.content.clone(),
            _ => String::new(),
        };
        crate::provider::live::write_live(app, &write_provider, &write_snippet)?;
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
    crate::provider::snippet::validate_snippet(app, &json)?;
    let snippet = CommonConfigSnippet {
        enabled,
        content: json,
    };
    state.config.update(|c| c.set_snippet(app, snippet))?;
    Ok(state.config.get().snippet_for(app))
}

/// TOML 片段整理（「整理」按钮）：taplo 保留注释地格式化（codex/grok 片段）。
/// 纯展示操作，不落盘、不校验身份键——只把文本排版成可读多行。
#[tauri::command]
#[specta::specta]
pub async fn format_toml_cmd(text: String) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || Ok(crate::provider::snippet::format_toml(&text)))
        .await
        .map_err(|e| AppError::Internal(format!("format_toml task failed: {e}")))?
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
/// 兼容 `GET /v1/models`，gemini 走 Google 原生 `GET /v1beta/models`。WebView
/// fetch 撞 CORS，所以请求由后端发（ureq）。claude / codex 路径里 `models_url`
/// 非空时精确覆写候选列表（只试这一个）；否则对 baseURL 构造候选 URL（版本段
/// 识别 + 兼容子路径剥离，见 `provider::model_fetch::candidate_models_urls`），
/// 按序尝试首个成功。gemini 路径端点形状固定（`gemini_models_url` 构造单一
/// URL），`models_url` 不参与。错误串带稳定前缀标签（AUTH_FAILED /
/// ENDPOINT_CLOSED / TIMEOUT / BAD_FORMAT / NETWORK），两条路径同一套标签，
/// 前端按标签分桶提示。
#[tauri::command]
#[specta::specta]
pub async fn fetch_models_cmd(
    app: App,
    base_url: String,
    api_key: String,
    models_url: Option<String>,
) -> AppResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        if app == App::Gemini {
            crate::provider::model_fetch::fetch_gemini_models(&base_url, &api_key)
        } else {
            crate::provider::model_fetch::fetch_models(&base_url, &api_key, models_url.as_deref())
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("fetch_models task failed: {e}")))?
}

// ---------------- 附加模式（OpenCode）写盘命令 ----------------

/// 附加模式核心动作（OpenCode）：把 provider ensure-in-live——写进 opencode.json
/// 同时设 `meta.liveManaged = true` 并落库。key 由 `live_opencode::derive_live_key`
/// 派生（优先沿用 meta.liveKey，改名不重算；首次按 name slugify，空 → 回落 id）。
/// **不取消其它 provider、不碰 active_providers**（附加模式无唯一激活）。返回
/// 更新了 meta 的 provider。
fn ensure_opencode_in_live(store: &Store, provider: Provider) -> AppResult<Provider> {
    let path = live_opencode::opencode_config_path()?;
    let live_text = live::read_live_settings(&path)?;
    let key =
        live_opencode::derive_live_key(&provider.name, &provider.id, &provider.meta, &live_text);
    live_opencode::set_opencode_provider(&path, &key, &provider.settings_config)?;
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, true)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式移除（OpenCode）：从 opencode.json 删 `provider.<liveKey>` + 设
/// `meta.liveManaged = false`（保留 liveKey，便于再加回来）。无 liveKey（从未写
/// 盘）→ 无操作，原样返回。
fn remove_opencode_from_live(store: &Store, provider: Provider) -> AppResult<Provider> {
    let Some(key) = live_opencode::meta_live_key(&provider.meta) else {
        return Ok(provider);
    };
    let path = live_opencode::opencode_config_path()?;
    live_opencode::remove_opencode_provider(&path, &key)?;
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, false)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式「从配置文件导入」：把 opencode.json 的 `provider.<key>` 反向导入 DB。
/// 读盘薄壳——核心逻辑在 [`import_opencode_from_live_text`]（可测，不碰文件系统）。
fn import_opencode_from_live(
    store: &Store,
    app: App,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let path = live_opencode::opencode_config_path()?;
    let live_text = live::read_live_settings(&path)?;
    import_opencode_from_live_text(store, app, &live_text, name_overrides)
}

/// 单激活应用「从 live 配置文件导入」（ADR-0012 泛化）：读该应用的 live 文件(s)
/// 反向解析为快照（至多 1 个），upsert 进库（按 name 去重）。opencode 走
/// [`import_opencode_from_live`]。返回写入条数。`name_overrides` = 预览列表里
/// 用户行内改过的名字（key → name；单激活 key == name）。
fn import_single_activate_from_live(
    store: &Store,
    app: App,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let Some(snap) = read_live_snapshot(app)? else {
        return Ok(0);
    };
    let mut snap = snap;
    if let Some(name) = name_overrides.get(&snap.name) {
        snap.name = name.clone();
    }
    import_live::upsert_by_name(store, app, &snap, &crate::time::now_iso())
}

/// 按 app 读 live 文件文本（顺序固定：claude=[settings.json]，codex=[config.toml,
/// auth.json]，gemini=[.env, settings.json]，grok=[config.toml]）。opencode 无单份
/// live 配置概念 → `None`。路径解析错误 → `Err`（与既有读盘语义一致）。快照与
/// 片段提取共用这一份「app → 路径」映射（单一事实来源）。
fn read_live_texts(app: App) -> AppResult<Option<Vec<String>>> {
    let paths: Vec<AppResult<PathBuf>> = match app {
        App::Claude => vec![live::claude_settings_path()],
        App::Codex => vec![
            live_codex::codex_config_path(),
            live_codex::codex_auth_path(),
        ],
        App::Gemini => vec![
            live_gemini::gemini_env_path(),
            live_gemini::gemini_settings_path(),
        ],
        App::Grok => vec![live_grok::grok_config_path()],
        App::OpenCode => return Ok(None),
    };
    let mut texts = Vec::with_capacity(paths.len());
    for p in paths {
        texts.push(live::read_live_settings(&p?)?);
    }
    Ok(Some(texts))
}

/// 按 app 读 live 文件(s) + 反向解析为快照。opencode → `unreachable`（走
/// opencode 专属路径）。
fn read_live_snapshot(app: App) -> AppResult<Option<import_live::LiveImportSnapshot>> {
    let Some(texts) = read_live_texts(app)? else {
        return Ok(None);
    };
    Ok(match app {
        App::Claude => import_live::claude_live_to_snapshot(&texts[0]),
        App::Codex => import_live::codex_live_to_snapshot(&texts[0], &texts[1]),
        App::Gemini => import_live::gemini_live_to_snapshot(&texts[0], &texts[1]),
        App::Grok => import_live::grok_live_to_snapshot(&texts[0]),
        App::OpenCode => unreachable!("opencode 走 import_opencode_from_live"),
    })
}

/// 单激活应用「从 live 导入」预览：快照 → 0/1 条 entry（key = name，is_new 按
/// name 是否已存在）。文件缺失/无可导入 → Ready 空（与 opencode 的空 provider
/// 段同语义；opencode 的 Missing 状态保留给它的路径不存在场景）。
fn preview_single_activate(store: &Store, app: App) -> AppResult<LiveImportPreview> {
    let existing_names: HashSet<String> = store
        .list_providers_for(app)?
        .into_iter()
        .map(|p| p.name)
        .collect();
    let Some(snap) = read_live_snapshot(app)? else {
        return Ok(LiveImportPreview::Ready { entries: vec![] });
    };
    Ok(LiveImportPreview::Ready {
        entries: vec![LiveImportPreviewEntry {
            key: snap.name.clone(),
            name: snap.name.clone(),
            base_url: snap.base_url.clone(),
            has_secret: snap.has_secret,
            is_new: !existing_names.contains(&snap.name),
            snippet_candidates: snap.snippet_candidates,
        }],
    })
}

/// import 的核心逻辑（可测）：给定 opencode.json 文本，把 `provider.<key>` 反向
/// 导入 DB。每个 key → 一条 Provider：已存在同 liveKey → 更新 settings_config +
/// name（保留 id/展示字段）；否则新建（空 id 交 save_provider 自动生成 hex id +
/// sort_index + updated_at）。反复 import 按 liveKey 去重，不产生重复。返回导入/
/// 更新条数。
fn import_opencode_from_live_text(
    store: &Store,
    app: App,
    live_text: &str,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let entries = live_opencode::provider_entries(live_text);
    if entries.is_empty() {
        return Ok(0);
    }
    // 现有 provider 按 liveKey 索引——按配置文件原 key 判「已存在」。
    let mut by_live_key: HashMap<String, Provider> = HashMap::new();
    for p in store.list_providers_for(app)? {
        if let Some(k) = live_opencode::meta_live_key(&p.meta) {
            by_live_key.insert(k, p);
        }
    }
    let mut count = 0u32;
    for (key, entry) in entries {
        let settings_config = serde_json::to_string(&entry)?;
        // 预览列表的行内改名优先（key → name 覆盖），否则 entry.name，缺 → key。
        let display_name = name_overrides
            .get(&key)
            .cloned()
            .unwrap_or_else(|| {
                entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&key)
                    .to_string()
            });
        let provider = match by_live_key.get(&key) {
            Some(existing) => Provider {
                name: display_name,
                settings_config,
                meta: live_opencode::with_meta_live_state(&existing.meta, &key, true)?,
                ..existing.clone()
            },
            None => Provider {
                id: String::new(),
                name: display_name,
                website_url: String::new(),
                category: ProviderCategory::Custom,
                app,
                icon: String::new(),
                icon_color: String::new(),
                sort_index: 0,
                notes: String::new(),
                settings_config,
                meta: live_opencode::with_meta_live_state("", &key, true)?,
                updated_at: String::new(),
            },
        };
        store.save_provider(provider)?;
        count += 1;
    }
    Ok(count)
}

/// 附加模式「添加」按钮：把 provider ensure-in-live（写进 opencode.json + 设
/// liveManaged=true）。仅附加模式 app 有意义（单激活用 switch_provider_cmd）。
#[tauri::command]
#[specta::specta]
pub async fn add_provider_to_live_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<Provider> {
    let store = state.store.clone();
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<Provider> {
        let provider = store
            .get_provider(app, &id)?
            .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
        ensure_opencode_in_live(&store, provider)
    })
    .await
    .map_err(|e| AppError::Internal(format!("add_provider_to_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 附加模式「移除」按钮：从 opencode.json 删 provider（设 liveManaged=false，DB
/// 记录保留，随时再加回来）。
#[tauri::command]
#[specta::specta]
pub async fn remove_provider_from_live_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<Provider> {
    let store = state.store.clone();
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<Provider> {
        let provider = store
            .get_provider(app, &id)?
            .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
        remove_opencode_from_live(&store, provider)
    })
    .await
    .map_err(|e| AppError::Internal(format!("remove_provider_from_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 附加模式「从配置文件导入」按钮：把现有 opencode.json 的 `provider.*` 反向拉进
/// cc one DB。返回导入/更新条数。
#[tauri::command]
#[specta::specta]
pub async fn import_providers_from_live_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    name_overrides: HashMap<String, String>,
) -> AppResult<u32> {
    let store = state.store.clone();
    let count = tauri::async_runtime::spawn_blocking(move || -> AppResult<u32> {
        // opencode 附加模式多条目共存；单激活应用一份 live → 至多一个快照。
        if app == App::OpenCode {
            import_opencode_from_live(&store, app, &name_overrides)
        } else {
            import_single_activate_from_live(&store, app, &name_overrides)
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("import_providers_from_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(count)
}

// ---------------- 附加模式（OpenCode）导入预览 ----------------

/// 「从 live 配置导入」的预览载荷（opencode 与单激活应用共用，ADR-0012）。
/// 文件不存在 → `Missing`（带完整路径，前端展示，仅 opencode 路径）；存在 →
/// 将导入的条目列表（空 = 无 provider 段 / 无可导入）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LiveImportPreview {
    Missing {
        path: String,
    },
    Ready {
        entries: Vec<LiveImportPreviewEntry>,
    },
}

/// 一条将导入的供应商预览。**密钥绝不进预览载荷**——只有布尔
/// `has_secret`，apiKey / headers 值不跨边界（见 `secret_in_entry`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LiveImportPreviewEntry {
    /// `provider.<key>`（opencode）或 name（单激活应用），即导入后的去重键。
    pub key: String,
    /// entry.name 优先，缺 → key（与导入的 display_name 规则一致）。
    pub name: String,
    /// options.baseURL（opencode）或 live 里 base_url（单激活），缺 → ""。
    pub base_url: String,
    /// options.apiKey / options.headers（opencode）或 settingsConfig 携带凭据
    /// （单激活）任一非空。
    pub has_secret: bool,
    /// DB 无此 key → 新建；有 → 更新（与导入的判定一致）。
    pub is_new: bool,
    /// 可共享键候选（单激活应用导入后可提取为通用片段；opencode 无此概念 → 空）。
    pub snippet_candidates: Vec<String>,
}

/// 预览「从 opencode.json 导入」：读盘薄壳——核心逻辑在
/// [`preview_opencode_import_text`]（可测，不碰文件系统/DB）。文件不存在 →
/// `Missing`（不报错：与导入命令「空文件 → 0 条」同属正常路径）。
fn preview_opencode_import(store: &Store, app: App, path: &Path) -> AppResult<LiveImportPreview> {
    if !path.exists() {
        return Ok(LiveImportPreview::Missing {
            path: path.display().to_string(),
        });
    }
    let live_text = live::read_live_settings(path)?;
    let existing_keys: HashSet<String> = store
        .list_providers_for(app)?
        .iter()
        .filter_map(|p| live_opencode::meta_live_key(&p.meta))
        .collect();
    Ok(LiveImportPreview::Ready {
        entries: preview_opencode_import_text(&live_text, &existing_keys),
    })
}

/// preview 核心逻辑（可测）：复用 [`live_opencode::provider_entries`]（单一事实
/// 来源，不重新解析），把 `provider.<key>` 转成预览条目。existing_keys 由调用
/// 方从 DB 收集——「新建 vs 更新」判定与 import（meta.liveKey 集合）一致。
fn preview_opencode_import_text(
    live_text: &str,
    existing_keys: &HashSet<String>,
) -> Vec<LiveImportPreviewEntry> {
    live_opencode::provider_entries(live_text)
        .into_iter()
        .map(|(key, entry)| LiveImportPreviewEntry {
            key: key.clone(),
            name: entry
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&key)
                .to_string(),
            base_url: entry
                .pointer("/options/baseURL")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            has_secret: secret_in_entry(&entry),
            is_new: !existing_keys.contains(&key),
            snippet_candidates: vec![],
        })
        .collect()
}

/// entry 里是否携带凭据（只出布尔，不回取密钥值）：options.apiKey 非空，或
/// options.headers 任一值非空（headers 可携带 Authorization 等认证头）。
fn secret_in_entry(entry: &serde_json::Value) -> bool {
    if entry
        .pointer("/options/apiKey")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    entry
        .pointer("/options/headers")
        .and_then(|h| h.as_object())
        .is_some_and(|m| {
            m.values()
                .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        })
}

/// 「从本机配置文件导入」预览：只读命令，按 app 分派（opencode 走读盘 +
/// Missing 状态；单激活应用走快照解析），返回将导入的供应商（名称/端点/是否
/// 含密钥/新建或更新/片段候选）。确认导入仍走 import_providers_from_live_cmd。
/// 不 emit、不失效任何 tag。
#[tauri::command]
#[specta::specta]
pub async fn preview_live_import_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<LiveImportPreview> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // opencode 走读盘 + Missing 状态；单激活应用走快照解析（无 Missing——
        // 文件缺失即无可导入，Ready 空）。
        if app == App::OpenCode {
            let path = live_opencode::opencode_config_path()?;
            preview_opencode_import(&store, app, &path)
        } else {
            preview_single_activate(&store, app)
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("preview_opencode_import task failed: {e}")))?
}

// ---------------- 导入后提取通用配置片段（T6）----------------

/// 按 app 读 live 文件(s)，提取「可共享键」为片段内容（无可提取 → None）。
/// opencode 无片段概念 → None。gemini 只需 env（settings.json 的非受控键进片段
/// 零效果，见 `gemini_extract_snippet`）。
fn read_live_snippet_extract(app: App) -> AppResult<Option<String>> {
    let Some(texts) = read_live_texts(app)? else {
        return Ok(None);
    };
    Ok(match app {
        App::Claude => import_live::claude_extract_snippet(&texts[0]),
        App::Gemini => import_live::gemini_extract_snippet(&texts[0]),
        App::Codex => import_live::codex_extract_snippet(&texts[0]),
        App::Grok => import_live::grok_extract_snippet(&texts[0]),
        App::OpenCode => unreachable!("opencode 无片段概念（read_live_texts 已返回 None）"),
    })
}

/// 导入后「提取为通用片段」（T6，ADR-0012）：读该应用 live 配置的可共享键，
/// 合并进现有片段（已有键不覆盖，沿用 ADR-0010 只补缺失），启用片段。无可
/// 提取 → 现有片段原样（enabled 不变，不碰用户停用状态）。非静默——前端先检测
/// 候选、用户确认才调本命令。返回更新后的片段。
#[tauri::command]
#[specta::specta]
pub fn extract_snippet_from_live_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<CommonConfigSnippet> {
    let current = state.config.get().snippet_for(app);
    let Some(extracted) = read_live_snippet_extract(app)? else {
        return Ok(current);
    };
    let content = import_live::merge_extracted_snippet(app, &current.content, &extracted)?;
    let snippet = CommonConfigSnippet {
        enabled: true,
        content,
    };
    state.config.update(|c| c.set_snippet(app, snippet))?;
    Ok(state.config.get().snippet_for(app))
}

// ---------------- 从 CC-Switch 导入供应商 ----------------

/// 定位本机 CC-Switch 配置：`custom` 优先；否则回退顺序 = 默认 `~/.cc-switch/
/// cc-switch.db` →（Windows）legacy `$HOME/.cc-switch/cc-switch.db` → 旧版
/// `~/.cc-switch/config.json`。任一存在即用。都找不到 → 明确错误（前端友好提示）。
fn locate_ccswitch_config(custom: &Option<String>) -> AppResult<PathBuf> {
    if let Some(p) = custom {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
        return Err(AppError::Config(format!(
            "指定的 CC-Switch 配置位置不存在: {p}"
        )));
    }
    let home = dirs::home_dir().ok_or_else(|| AppError::Config("无法定位 home 目录".into()))?;
    let cs_dir = home.join(".cc-switch");
    let db = cs_dir.join("cc-switch.db");
    if db.exists() {
        return Ok(db);
    }
    #[cfg(windows)]
    {
        // 兼容 v3.10.3 误用 HOME 环境变量的旧版本（仅 Windows）。
        if let Ok(h) = std::env::var("HOME") {
            let legacy = PathBuf::from(h).join(".cc-switch").join("cc-switch.db");
            if legacy.exists() {
                return Ok(legacy);
            }
        }
    }
    let json = cs_dir.join("config.json");
    if json.exists() {
        return Ok(json);
    }
    Err(AppError::Config(
        "未检测到本机 CC-Switch 配置，请确认 CC-Switch 已安装，或手动指定配置位置".into(),
    ))
}

/// 读 CC-Switch 配置（SQLite 或旧 JSON）→ (独立供应商, 统一供应商)。SQLite 以只读
/// 连接打开（不加写锁，避免干扰可能正在运行的 CC-Switch），读后连接析构关闭。
fn read_ccswitch_source(
    custom: &Option<String>,
) -> AppResult<(
    Vec<import_ccswitch::CcSwitchProvider>,
    Vec<import_ccswitch::UniversalProvider>,
)> {
    let path = locate_ccswitch_config(custom)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "db" {
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let providers = import_ccswitch::read_providers_from_db(&conn)?;
        let universals = import_ccswitch::read_universals_from_db(&conn)?;
        Ok((providers, universals))
    } else {
        let text = std::fs::read_to_string(&path)?;
        import_ccswitch::parse_legacy_json(&text)
    }
}

/// 「从 CC-Switch 导入」按钮：定位本机 CC-Switch 配置 → 读 + 转换供应商 → 复用
/// `apply_import`（merge / overwrite）写本机库。**单应用语境**：`app` 是当前
/// 视图的应用，只搬该应用的供应商（claude 视图不冒出 codex 供应商，见
/// ADR-0012）。代理 / OAuth / 不支持应用的供应商跳过并进报告明细。找不到配置
/// → 明确错误（前端展示友好提示）。
#[tauri::command]
#[specta::specta]
pub async fn import_from_ccswitch_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    mode: ProviderImportMode,
    db_path: Option<String>,
) -> AppResult<import_ccswitch::CcSwitchImportReport> {
    let store = state.store.clone();
    let report = tauri::async_runtime::spawn_blocking(
        move || -> AppResult<import_ccswitch::CcSwitchImportReport> {
            let now = crate::time::now_iso();
            let (providers, universals) = read_ccswitch_source(&db_path)?;
            let (imported, skipped) =
                import_ccswitch::collect_ccswitch_imports(&providers, &universals, app, &now);
            // 复用 export_import 的写库路径：序列化成导出文档喂 apply_import（不新造
            // 冲突逻辑——冲突键 (app, id)、只写本机 DB 由它守住）。
            let doc = export_import::export_document(&imported, true, &now)?;
            let apply = export_import::apply_import(&store, &doc, mode)?;
            Ok(import_ccswitch::CcSwitchImportReport {
                imported: apply.imported,
                merge_skipped: apply.skipped,
                proxy_skipped: skipped,
            })
        },
    )
    .await
    .map_err(|e| AppError::Internal(format!("import_from_ccswitch task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::mem;

    /// 一份带两个 provider（一个带 name、一个不带）+ 顶层用户字段的 opencode.json。
    fn opencode_live_json() -> &'static str {
        r#"{
          "model": "deepseek/deepseek-chat",
          "provider": {
            "deepseek": {
              "npm": "@ai-sdk/openai-compatible",
              "name": "DeepSeek",
              "options": { "baseURL": "https://api.deepseek.com", "apiKey": "sk-x" }
            },
            "kimi": {
              "npm": "@ai-sdk/openai-compatible",
              "options": { "baseURL": "https://api.moonshot.cn" }
            }
          }
        }"#
    }

    /// 导入把 provider.<key> 反向落库：新建（空 id → 自动 hex）、liveKey=原 key、
    /// liveManaged=true；display name 取 entry.name，无 name 则取 key。
    #[test]
    fn import_creates_providers_with_live_key_and_managed_flag() {
        let s = mem();
        let n = import_opencode_from_live_text(
            &s,
            App::OpenCode,
            opencode_live_json(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 2);
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        assert_eq!(providers.len(), 2);
        let by_name: HashMap<String, Provider> = providers
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();
        // 带 name 的 → 用 name。
        let ds = by_name.get("DeepSeek").expect("entry.name 作 display name");
        assert_eq!(
            live_opencode::meta_live_key(&ds.meta).as_deref(),
            Some("deepseek"),
            "liveKey = 配置文件原 key"
        );
        assert_eq!(live_opencode::meta_live_managed(&ds.meta), Some(true));
        // 不带 name 的 → 用 key。
        let kimi = by_name.get("kimi").expect("无 name → key 作 display name");
        assert_eq!(
            live_opencode::meta_live_key(&kimi.meta).as_deref(),
            Some("kimi")
        );
        // settingsConfig 是 entry 子树（npm/options）。
        let sc: serde_json::Value = serde_json::from_str(&ds.settings_config).unwrap();
        assert_eq!(sc["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(sc["options"]["baseURL"], "https://api.deepseek.com");
    }

    /// 反复 import 同一文件 → 按 liveKey 匹配更新，不产生重复。
    #[test]
    fn import_updates_existing_same_live_key_no_duplicate() {
        let s = mem();
        import_opencode_from_live_text(&s, App::OpenCode, opencode_live_json(), &HashMap::new())
            .unwrap();
        let n = import_opencode_from_live_text(&s, App::OpenCode, opencode_live_json(), &HashMap::new())
            .unwrap();
        assert_eq!(n, 2, "第二次仍处理 2 条（按 liveKey 更新）");
        assert_eq!(
            s.list_providers_for(App::OpenCode).unwrap().len(),
            2,
            "不产生重复"
        );
    }

    /// 行内改名覆盖：nameOverrides（key → name）优先于 entry.name / key。
    #[test]
    fn import_respects_name_overrides() {
        let s = mem();
        let overrides = HashMap::from([("deepseek".to_string(), "DS 直连".to_string())]);
        let n = import_opencode_from_live_text(&s, App::OpenCode, opencode_live_json(), &overrides)
            .unwrap();
        assert_eq!(n, 2);
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        let ds = providers
            .iter()
            .find(|p| live_opencode::meta_live_key(&p.meta).as_deref() == Some("deepseek"))
            .expect("deepseek 存在");
        assert_eq!(ds.name, "DS 直连", "覆盖名优先于 entry.name");
        // 未被覆盖的 key 仍走 entry.name / key 规则。
        let kimi = providers
            .iter()
            .find(|p| live_opencode::meta_live_key(&p.meta).as_deref() == Some("kimi"))
            .expect("kimi 存在");
        assert_eq!(kimi.name, "kimi");
    }

    /// 无 provider 段 → 0 条（顶层用户字段 model 等被忽略，不报错）。
    #[test]
    fn import_empty_providers_section_is_zero() {
        let s = mem();
        let n =
            import_opencode_from_live_text(&s, App::OpenCode, r#"{"model":"x"}"#, &HashMap::new())
                .unwrap();
        assert_eq!(n, 0);
        assert!(s.list_providers_for(App::OpenCode).unwrap().is_empty());
    }

    // ---------------- 单激活应用「从 live 导入」（ADR-0012 泛化）----------------

    /// claude live → 快照 → upsert：按 name（base_url host）新建 Provider。
    #[test]
    fn single_activate_import_claude_live_creates_provider() {
        let s = mem();
        let snap = import_live::claude_live_to_snapshot(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"kimi"}}"#,
        )
        .unwrap();
        let n =
            import_live::upsert_by_name(&s, App::Claude, &snap, "2026-08-13T00:00:00Z").unwrap();
        assert_eq!(n, 1);
        let providers = s.list_providers_for(App::Claude).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "moonshot", "注册域去 TLD（host_of 规则）");
        let sc: serde_json::Value = serde_json::from_str(&providers[0].settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "kimi");
    }

    /// 反复导入同 name → 更新不重复（单激活的 liveKey 替代：按 name 去重）。
    #[test]
    fn single_activate_import_dedupes_by_name() {
        let s = mem();
        let snap =
            import_live::claude_live_to_snapshot(r#"{"env":{"ANTHROPIC_MODEL":"m1"}}"#).unwrap();
        import_live::upsert_by_name(&s, App::Claude, &snap, "t1").unwrap();
        import_live::upsert_by_name(&s, App::Claude, &snap, "t2").unwrap();
        assert_eq!(
            s.list_providers_for(App::Claude).unwrap().len(),
            1,
            "同 name 不产生重复"
        );
    }

    /// 无可导入内容（无受控键）→ 0 条。
    #[test]
    fn single_activate_import_empty_live_is_zero() {
        let s = mem();
        let snap = import_live::claude_live_to_snapshot(r#"{"permissions":{"allow":["Bash"]}}"#);
        assert!(snap.is_none(), "无受控内容 → 无可导入");
        assert!(s.list_providers_for(App::Claude).unwrap().is_empty());
    }

    // ---------------- 导入预览 ----------------

    /// 预览提取 name / endpoint / 密钥布尔：带 name 用 name，缺 name 用 key；
    /// baseURL 取 options.baseURL；apiKey 存在 → has_secret=true。键序字母序。
    #[test]
    fn preview_lists_entries_with_name_endpoint_and_secret() {
        let entries = preview_opencode_import_text(opencode_live_json(), &HashSet::new());
        assert_eq!(entries.len(), 2, "字母序：deepseek 先于 kimi");
        let ds = &entries[0];
        assert_eq!(ds.key, "deepseek");
        assert_eq!(ds.name, "DeepSeek", "entry.name 作显示名");
        assert_eq!(ds.base_url, "https://api.deepseek.com");
        assert!(ds.has_secret, "options.apiKey 非空 → 含密钥");
        assert!(ds.is_new, "DB 无此 liveKey → 新建");
        let kimi = &entries[1];
        assert_eq!(kimi.key, "kimi");
        assert_eq!(kimi.name, "kimi", "无 name → key 作显示名");
        assert_eq!(kimi.base_url, "https://api.moonshot.cn");
        assert!(!kimi.has_secret, "无 apiKey → 不含密钥");
    }

    /// 「新建 vs 更新」判定与导入一致：existing_keys 按 liveKey 集合判定。
    #[test]
    fn preview_classifies_new_vs_update() {
        let existing: HashSet<String> = ["deepseek".to_string()].into_iter().collect();
        let entries = preview_opencode_import_text(opencode_live_json(), &existing);
        assert!(!entries[0].is_new, "已有同 liveKey → 更新");
        assert!(entries[1].is_new, "无此 liveKey → 新建");
    }

    /// 防泄漏回归护栏：预览载荷序列化后绝不包含密钥值（apiKey 只出布尔）。
    #[test]
    fn preview_output_never_contains_secret_value() {
        let entries = preview_opencode_import_text(opencode_live_json(), &HashSet::new());
        let json = serde_json::to_string(&entries).expect("preview entries serialize");
        assert!(!json.contains("sk-x"), "preview 载荷不得携带密钥值: {json}");
    }

    /// headers 也能携带凭据（Authorization 等）→ 计入 has_secret；空值不算。
    #[test]
    fn preview_detects_headers_secret() {
        let live = r#"{
          "provider": {
            "h1": { "options": { "headers": { "Authorization": "Bearer abc" } } },
            "h2": { "options": { "headers": { "X-Empty": "" } } },
            "h3": { "options": { "apiKey": "" } }
          }
        }"#;
        let entries = preview_opencode_import_text(live, &HashSet::new());
        assert_eq!(entries.len(), 3);
        assert!(entries[0].has_secret, "headers 非空值 → 含密钥");
        assert!(!entries[1].has_secret, "headers 空值 → 不含");
        assert!(!entries[2].has_secret, "apiKey 空串 → 不含");
    }

    /// 无 provider 段 / 损坏 JSON5 / 非对象根 → 空 Vec（与导入「静默 0 条」一致，
    /// preview 与 import 语义不得分叉）。
    #[test]
    fn preview_empty_or_unparseable_live_is_empty() {
        for live in [r#"{"model":"x"}"#, "{bad", "[1,2]", ""] {
            let entries = preview_opencode_import_text(live, &HashSet::new());
            assert!(entries.is_empty(), "输入 {live:?} 应 → 空 Vec");
        }
    }

    /// 薄壳：文件不存在 → Missing 变体（带完整路径），不是 Err。
    #[test]
    fn preview_shell_missing_file_is_missing_variant() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        match preview_opencode_import(&s, App::OpenCode, &path).expect("missing is Ok") {
            LiveImportPreview::Missing { path: shown } => {
                assert_eq!(shown, path.display().to_string());
            }
            LiveImportPreview::Ready { .. } => panic!("缺文件应 Missing"),
        }
    }

    /// 薄壳：文件存在 → Ready + 条目（经真实读盘路径）。
    #[test]
    fn preview_shell_ready_with_seeded_file() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        std::fs::write(&path, opencode_live_json()).expect("write live file");
        match preview_opencode_import(&s, App::OpenCode, &path).expect("ready is Ok") {
            LiveImportPreview::Ready { entries } => {
                assert_eq!(entries.len(), 2);
                assert!(entries.iter().all(|e| e.is_new), "空 DB → 全部新建");
            }
            LiveImportPreview::Missing { .. } => panic!("文件存在应 Ready"),
        }
    }
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
