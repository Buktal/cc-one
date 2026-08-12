//! cc one Tauri backend library.
//!
//! Module tree: config / db / source_parser / ingest / artifact / session_snapshot
//! / jsonl / collect / pricing / sync / proxy / commands / window_geom, behind a tauri-specta typed
//! contract. First start bootstraps the local data dir + deviceId and defaults
//! to Standalone
//!.

use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(debug_assertions)]
use specta_typescript::Typescript;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};
use tauri_specta::Builder;

mod collect;
mod commands;
mod config;
mod db;
mod devices;
mod error;
mod library;
mod model;
mod pricing;
mod provider;
mod proxy;
mod sessions;
mod source_parser;
mod sync;
mod time;
mod window_geom;

use commands::AppState;
use config::{ConfigData, ConfigStore};
use db::Store;

/// Assemble the tauri-specta builder with all typed commands.
fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![
        commands::get_app_info,
        commands::set_sync_repo,
        commands::clear_sync_repo,
        commands::set_display_name,
        commands::set_device_display_name,
        commands::forget_device,
        commands::collect_now,
        commands::sync_now,
        commands::rebill_zero_cost,
        commands::query_usage_stats,
        commands::query_usage_trend,
        commands::query_usage_logs,
        commands::count_usage_logs,
        commands::query_models,
        commands::query_distinct_sources,
        commands::query_distinct_models,
        commands::list_devices,
        commands::list_pricing,
        commands::save_pricing_entry,
        commands::delete_pricing_entry,
        commands::reload_pricing_from_file,
        commands::save_pricing_to_file,
        commands::fetch_litellm_pricing,
        commands::get_preferences,
        commands::set_close_behavior,
        commands::set_collect_interval,
        commands::set_push_interval,
        commands::set_language,
        commands::set_tray_theme,
        commands::set_lightweight_expand,
        commands::set_skin,
        commands::verify_sync_repo,
        commands::confirm_close,
        commands::scan_library,
        commands::upload_to_library,
        commands::export_from_library,
        commands::delete_from_library,
        commands::rename_in_library,
        commands::read_library_text,
        commands::library_device_summary,
        commands::query_sessions_cmd,
        commands::count_sessions_cmd,
        commands::get_session_transcript_cmd,
        commands::set_session_favorited_cmd,
        commands::set_session_custom_title_cmd,
        commands::set_session_local_group_cmd,
        commands::set_session_synced_group_cmd,
        commands::list_local_groups_cmd,
        commands::create_local_group_cmd,
        commands::rename_local_group_cmd,
        commands::delete_local_group_cmd,
        commands::reorder_local_groups_cmd,
        commands::list_synced_groups_cmd,
        commands::create_synced_group_cmd,
        commands::rename_synced_group_cmd,
        commands::delete_synced_group_cmd,
        commands::reorder_synced_groups_cmd,
        commands::list_groups_cmd,
        commands::list_providers_cmd,
        commands::save_provider_cmd,
        commands::delete_provider_cmd,
        commands::reorder_providers_cmd,
        commands::switch_provider_cmd,
        commands::get_active_provider_cmd,
        commands::get_common_config_snippet_cmd,
        commands::set_common_config_snippet_cmd,
        commands::export_providers_cmd,
        commands::import_providers_cmd,
        commands::fetch_models_cmd,
        commands::add_provider_to_live_cmd,
        commands::remove_provider_from_live_cmd,
        commands::import_providers_from_live_cmd,
        commands::import_from_ccswitch_cmd,
        window_geom::dock_window_right,
        window_geom::center_window,
        window_geom::set_window_rect,
    ])
}

/// Tray "Show" label for the active display language. The tray menu
/// items are the ONLY user-facing Rust strings — all other UI text is frontend
/// i18n — so this and [`quit_label`] are rebuilt on a live language change.
pub(crate) fn show_label(lang: config::Language) -> &'static str {
    match lang {
        config::Language::En => "Show",
        config::Language::Zh => "显示",
        config::Language::Ja => "表示",
    }
}

/// Tray "Quit" label for the active display language.
pub(crate) fn quit_label(lang: config::Language) -> &'static str {
    match lang {
        config::Language::En => "Quit",
        config::Language::Zh => "退出",
        config::Language::Ja => "終了",
    }
}

/// (Re)build the tray menu, localized to `lang`. Called at setup
/// and from `set_language` on a language change so both items track the
/// language without an app restart.
///
/// Two items: "Show" surfaces the dashboard; "Quit" exits.
/// The Show entry is the ONLY reliable restore path on Linux, where
/// libappindicator does not emit tray click events — so the left-click handler
/// (`on_tray_icon_event` below) is dead code there and the menu Show item is
/// the fallback. On Windows/macOS left-click stays the primary path and Show is
/// a convenience.
pub(crate) fn tray_menu_for(
    app: &tauri::AppHandle,
    lang: config::Language,
) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", show_label(lang), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label(lang), true, None::<&str>)?;
    Menu::with_items(app, &[&show, &quit])
}

/// Surface the main window from the tray: show + unminimize + focus, then tell
/// the frontend to morph out of lightweight mode so the FULL
/// dashboard is shown. Shared by the tray menu "Show" entry and the tray
/// left-click handler. `unminimize` covers the GNOME/Mutter case where a hidden
/// window needs both show and unminimize to reliably take focus.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        let _ = app.emit("tray-show-main", ());
    }
}

/// Export TypeScript bindings to the frontend `src/types/generated/`
///. Dev builds only; skipped for release binaries. Path is resolved
/// from `CARGO_MANIFEST_DIR` so it is correct regardless of the runtime CWD.
fn export_bindings(builder: &Builder<tauri::Wry>) {
    #[cfg(debug_assertions)]
    {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("src")
            .join("types")
            .join("generated")
            .join("bindings.ts");
        builder
            .export(
                Typescript::default().header("// tauri-specta generated. Do not edit manually."),
                path,
            )
            .expect("Failed to export tauri-specta TypeScript bindings");
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = builder;
    }
}

/// One scheduler action returned by [`plan_tick`]. The list order IS the
/// execution order — `Collect` always precedes `Sync` (see [`plan_tick`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickAction {
    /// Parse Sources → Local Store (+ JSONL Artifact). No network.
    Collect,
    /// One pull+push round (Synced only). The cadence is the retry — no
    /// per-tick retry here.
    Sync,
}

/// Pure per-tick decision for the background scheduler: given the current
/// time, the two deadlines, and the live config, return the actions to run
/// this tick (in order) plus the updated deadlines.
///
/// **Encodes the collect-before-sync invariant.** When both deadlines fire in
/// the same tick, `Collect` is always placed before `Sync` in the returned
/// `Vec`: collect's JSONL `writeln!` has fully flushed by the time it returns,
/// so the subsequent `git add` snapshots complete lines only (no half-line
/// race) — safe only because the scheduler runs them serially on one thread.
/// This ordering used to live only in a prose comment next to the spawn; it is
/// now a table-testable `Vec` ordering.
///
/// Pure: no IO, no global state, no clock — `now` is a parameter, so the full
/// decision surface (independent intervals, collect-before-sync, `is_synced`
/// gate, interval clamping) is covered by `tests::plan_tick_table`.
///
/// A deadline that does not fire is returned unchanged, so in Standalone
/// (where `Sync` never fires) `next_push` stays at its initial value —
/// preserved exactly from the inline loop.
fn plan_tick(
    now: Instant,
    next_collect: Instant,
    next_push: Instant,
    cfg: &ConfigData,
) -> (Vec<TickAction>, Instant, Instant) {
    let collect_secs = cfg.collect_interval_secs.clamp(5, 3600) as u64;
    let push_secs = cfg.push_interval_secs.clamp(60, 7200) as u64;

    let mut actions = Vec::new();
    let mut new_collect = next_collect;
    let mut new_push = next_push;
    // Collect is evaluated and pushed first, so when both deadlines fire the
    // JSONL has flushed before git add runs. The invariant is this Vec order.
    if now >= next_collect {
        actions.push(TickAction::Collect);
        new_collect = now + Duration::from_secs(collect_secs);
    }
    if now >= next_push && cfg.is_synced() {
        actions.push(TickAction::Sync);
        new_push = now + Duration::from_secs(push_secs);
    }
    (actions, new_collect, new_push)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();
    export_bindings(&builder);

    // Headless binding-generation mode: regenerate `bindings.ts` then exit
    // without launching a window (CI / `VAULTONE_GEN_BINDINGS=1 cargo run`).
    // The real app exe carries the Common-Controls v6 manifest from
    // tauri-build, so generation must run through this bin, not `cargo test`.
    if std::env::var("VAULTONE_GEN_BINDINGS").is_ok() {
        return;
    }

    // Boot: load config (bootstraps dir + deviceId),
    // open the Local Store (seeds pricing), register this device.
    let config = ConfigStore::load().expect("cc-one: failed to load local config");
    let store = Store::open(&config.paths().db).expect("cc-one: failed to open Local Store");
    {
        // Register this device in the Local Store and publish its name
        // artifact (covers both first run and an upgrade from a version that
        // predates device-name sync). Best-effort; the normal Git sync carries
        // the artifact. The registry lifecycle owns this now — see
        // `devices::register_self`.
        let _ = crate::devices::register_self(&store, &config);
        // Best-effort zero-cost top-up on boot: newly-seeded pricing
        // may price rows that were imported while the model was missing.
        let book = store.load_pricing_book().unwrap_or_else(|e| {
            eprintln!("[cc-one] boot rebill skipped: {e}");
            pricing::seed_book()
        });
        if let Err(e) = store.rebill_zero_cost(&book) {
            eprintln!("[cc-one] boot rebill failed: {e}");
        }
    }

    let state = AppState {
        store: Arc::new(store),
        config: Arc::new(config),
    };

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Native file open / save dialogs for Library export + "Add files".
        .plugin(tauri_plugin_dialog::init())
        // Auto-update: check / download / install signed packages
        // from GitHub Releases. Endpoint + pubkey live in tauri.conf.json
        // (plugins.updater); capability grants updater:default.
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Relaunch the app after an auto-update install.
        .plugin(tauri_plugin_process::init())
        .manage(state)
        .invoke_handler(builder.invoke_handler())
        .on_window_event(|window, event| {
            // Close→tray routing. Only the main window exists.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<AppState>();
                match state.config.get().close_behavior {
                    crate::config::CloseBehavior::Quit => {
                        // Let it close — triggers the exit flush (RunEvent).
                    }
                    crate::config::CloseBehavior::Minimize => {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                    crate::config::CloseBehavior::Ask => {
                        api.prevent_close();
                        let _ = window.app_handle().emit("close-requested", ());
                    }
                }
            }
        })
        .setup(|app| {
            let state: tauri::State<AppState> = app.state::<AppState>();

            // Startup pull (Synced only): covers the device-switch case.
            let store = state.store.clone();
            let config = state.config.clone();
            std::thread::spawn(move || {
                let cfg = config.get();
                if !cfg.is_synced() {
                    return;
                }
                let paths = config.paths();
                match crate::sync::pull_and_import(&store, &paths, &cfg) {
                    Ok(n) => eprintln!("[cc-one] startup pull imported {n} row(s)"),
                    Err(e) => eprintln!("[cc-one] startup pull failed: {e}"),
                }
            });

            // System tray: left-click shows the window; the menu is
            // Quit only — collect / sync live inside the window, not the tray.
            // The Quit label follows the persisted display language;
            // `set_language` rebuilds this menu on a live language change.
            let menu = tray_menu_for(app.handle(), state.config.get().language)?;
            // Tray starts on the dark badge (next-themes `defaultTheme="dark"`);
            // the frontend pushes `set_tray_theme` once it resolves the actual
            // theme on mount, so the icon follows light/system/dark afterwards.
            let _tray = TrayIconBuilder::with_id("main")
                .tooltip("CC One")
                .icon(
                    tauri::image::Image::from_bytes(include_bytes!("../icons/tray-dark.png"))
                        .expect("embedded tray icon"),
                )
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    // No-op on Linux: libappindicator does not emit tray click
                    // events, so the menu "Show" entry is the restore path there
                    // (see `tray_menu_for`). On Windows/macOS this left-click is
                    // the primary path.
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Background scheduler: collect and sync run on INDEPENDENT
            // intervals. Collect is short (seconds, local-only) so the dashboard
            // stays fresh; sync is longer (minutes, Synced only) — one pull+push
            // round per tick — so peer devices' usage lands here, this device's
            // goes up, and the Git history grows at a controlled rate.
            //
            // One thread, two deadlines, slept-to (not polled): each tick
            // re-reads the config snapshot and hands it to [`plan_tick`], so
            // Settings changes apply without restart.
            //
            // The collect-before-sync invariant — when BOTH deadlines fire in a
            // tick, collect's JSONL `writeln!` has fully flushed before the
            // subsequent `git add` runs (no half-line race), safe only because
            // execution is serial on this single thread — is encoded and tested
            // in [`plan_tick`]: `Collect` always precedes `Sync` in the returned
            // action list, and this closure just walks that list in order.
            //
            // Startup strategy: first collect fires immediately (next_collect =
            // start — dashboard is fresh on open); first sync is delayed one
            // push_interval (next_push = start + push_interval) so it cannot
            // race the startup pull's git-worktree ops. These are one-off
            // initializations; [`plan_tick`] owns the per-tick logic.
            let store = state.store.clone();
            let config = state.config.clone();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let start = Instant::now();
                let mut next_collect = start;
                let mut next_push = start
                    + Duration::from_secs(config.get().push_interval_secs.clamp(60, 7200) as u64);
                loop {
                    // Snapshot config once per tick (matches the original
                    // pre-sleep read so live Settings changes apply next tick).
                    let cfg = config.get();

                    // Sleep to the nearer deadline (not polled).
                    let now = Instant::now();
                    let next_deadline = next_collect.min(next_push);
                    if next_deadline > now {
                        std::thread::sleep(next_deadline - now);
                    }

                    let now = Instant::now();
                    let (actions, new_collect, new_push) =
                        plan_tick(now, next_collect, next_push, &cfg);
                    next_collect = new_collect;
                    next_push = new_push;
                    // Execute in returned order: Collect before Sync when both
                    // fire (the collect-before-sync invariant — Vec order, not prose).
                    for action in actions {
                        match action {
                            TickAction::Collect => {
                                if let Err(e) = collect::collect_into(&store, &config) {
                                    eprintln!("[cc-one] scheduled collect failed: {e}");
                                }
                                let _ = app_handle.emit("usage_changed", ());
                            }
                            TickAction::Sync => {
                                // One pull+push round (best-effort; the cadence
                                // is the retry — no explicit retry here). Pull
                                // lands peer devices' usage here, push sends
                                // this device's up.
                                let sr = collect::sync_round(&store, &config);
                                for e in &sr.errors {
                                    eprintln!("[cc-one] scheduled sync error: {e}");
                                }
                                let _ = app_handle.emit("usage_changed", ());
                            }
                        }
                    }
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // Exit flush: push any unpushed Artifact before quitting, covering the
        // close-A / open-B device switch. Synced only, best-effort.
        if let tauri::RunEvent::ExitRequested { .. } = event {
            let state: tauri::State<AppState> = app_handle.state::<AppState>();
            let cfg = state.config.get();
            let paths = state.config.paths();
            // Materialize any un-pushed days from the store, then push — covers
            // the close-A / open-B device switch (collect now writes the store,
            // not the Artifact, so the flush must recompute before pushing).
            crate::sync::push_usage_best_effort(&state.store, &paths, &cfg);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{plan_tick, TickAction};
    use crate::config::ConfigData;
    use std::time::{Duration, Instant};

    /// Synced-mode config: repo URL + PAT present ⇒ `is_synced()` true.
    fn synced_config() -> ConfigData {
        ConfigData {
            repo_url: Some("https://github.com/cc-one/test".to_string()),
            github_token: Some("github_pat_test".to_string()),
            collect_interval_secs: 30,
            push_interval_secs: 600,
            ..ConfigData::default()
        }
    }

    /// `plan_tick` table: every decision the scheduler makes per wake must be
    /// covered here — independent intervals, collect-before-sync ordering,
    /// `is_synced` gate, and unchanged-when-not-due deadlines.
    #[test]
    fn plan_tick_table() {
        let t0 = Instant::now();
        let collect_after = Duration::from_secs(30);
        let push_after = Duration::from_secs(600);

        struct Case {
            name: &'static str,
            now: Instant,
            next_collect: Instant,
            next_push: Instant,
            cfg: ConfigData,
            expect: Vec<TickAction>,
            expect_collect: Instant,
            expect_push: Instant,
        }

        let cases = vec![
            // 1. Only collect deadline reached (synced) → collect advances, push unchanged.
            Case {
                name: "only collect due (synced)",
                now: t0 + Duration::from_secs(100),
                next_collect: t0 + Duration::from_secs(100),
                next_push: t0 + Duration::from_secs(700),
                cfg: synced_config(),
                expect: vec![TickAction::Collect],
                expect_collect: t0 + Duration::from_secs(100) + collect_after,
                expect_push: t0 + Duration::from_secs(700),
            },
            // 2. Only push deadline reached (synced) → sync advances, collect unchanged.
            Case {
                name: "only push due (synced)",
                now: t0 + Duration::from_secs(200),
                next_collect: t0 + Duration::from_secs(300),
                next_push: t0 + Duration::from_secs(200),
                cfg: synced_config(),
                expect: vec![TickAction::Sync],
                expect_collect: t0 + Duration::from_secs(300),
                expect_push: t0 + Duration::from_secs(200) + push_after,
            },
            // 3. BOTH due (synced) → Collect BEFORE Sync (the invariant), both advance.
            Case {
                name: "both due (synced) — collect-before-sync",
                now: t0 + Duration::from_secs(500),
                next_collect: t0 + Duration::from_secs(100),
                next_push: t0 + Duration::from_secs(200),
                cfg: synced_config(),
                expect: vec![TickAction::Collect, TickAction::Sync],
                expect_collect: t0 + Duration::from_secs(500) + collect_after,
                expect_push: t0 + Duration::from_secs(500) + push_after,
            },
            // 4. Neither due → no action, both deadlines unchanged.
            Case {
                name: "neither due (synced)",
                now: t0 + Duration::from_secs(10),
                next_collect: t0 + Duration::from_secs(100),
                next_push: t0 + Duration::from_secs(700),
                cfg: synced_config(),
                expect: vec![],
                expect_collect: t0 + Duration::from_secs(100),
                expect_push: t0 + Duration::from_secs(700),
            },
            // 5. Push deadline reached but Standalone → Sync suppressed AND next_push
            //    is NOT advanced (the gate skips both the action and the reschedule).
            Case {
                name: "push due but standalone — sync suppressed",
                now: t0 + Duration::from_secs(300),
                next_collect: t0 + Duration::from_secs(400),
                next_push: t0 + Duration::from_secs(200),
                cfg: ConfigData::default(),
                expect: vec![],
                expect_collect: t0 + Duration::from_secs(400),
                expect_push: t0 + Duration::from_secs(200),
            },
            // 6. Both due but Standalone → Collect only; next_push unchanged.
            Case {
                name: "both due but standalone — collect only",
                now: t0 + Duration::from_secs(500),
                next_collect: t0 + Duration::from_secs(100),
                next_push: t0 + Duration::from_secs(200),
                cfg: ConfigData::default(),
                expect: vec![TickAction::Collect],
                expect_collect: t0 + Duration::from_secs(500) + collect_after,
                expect_push: t0 + Duration::from_secs(200),
            },
        ];

        for c in cases {
            let (actions, new_collect, new_push) =
                plan_tick(c.now, c.next_collect, c.next_push, &c.cfg);
            assert_eq!(actions, c.expect, "{}: actions", c.name);
            assert_eq!(new_collect, c.expect_collect, "{}: next_collect", c.name);
            assert_eq!(new_push, c.expect_push, "{}: next_push", c.name);
        }
    }

    /// Interval clamping must match the scheduler's `clamp` ranges exactly:
    /// collect ∈ [5, 3600], push ∈ [60, 7200].
    #[test]
    fn plan_tick_clamps_intervals() {
        let t0 = Instant::now();
        let far_future = t0 + Duration::from_secs(99_999);

        // collect floor 5s and ceiling 3600s (push held not-due).
        let mut cfg = synced_config();
        cfg.collect_interval_secs = 1;
        let (_, nc, _) = plan_tick(t0, t0, far_future, &cfg);
        assert_eq!(nc, t0 + Duration::from_secs(5), "collect floor 5s");
        cfg.collect_interval_secs = 50_000;
        let (_, nc, _) = plan_tick(t0, t0, far_future, &cfg);
        assert_eq!(nc, t0 + Duration::from_secs(3600), "collect ceiling 3600s");

        // push floor 60s and ceiling 7200s (collect held not-due, synced so Sync fires).
        cfg.collect_interval_secs = 30;
        cfg.push_interval_secs = 1;
        let (_, _, np) = plan_tick(t0, far_future, t0, &cfg);
        assert_eq!(np, t0 + Duration::from_secs(60), "push floor 60s");
        cfg.push_interval_secs = 50_000;
        let (_, _, np) = plan_tick(t0, far_future, t0, &cfg);
        assert_eq!(np, t0 + Duration::from_secs(7200), "push ceiling 7200s");
    }
}
