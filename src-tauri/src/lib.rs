//! cc one Tauri backend library.
//!
//! Module tree: boot / config / db / source_parser / ingest / artifact /
//! session_snapshot / jsonl / collect / pricing / sync / proxy / commands /
//! window_geom, behind a tauri-specta typed contract. First start bootstraps
//! the local data dir + deviceId and defaults to Standalone.
//!
//! This file is assembly only: the typed command registry, the plugin /
//! event-hook wiring, and the boot hand-off. The startup sequence itself is a
//! declarative step list in [`boot`] (each step carries a declared failure
//! criticality); the exit flush hangs off `boot::on_run_event` below.

#[cfg(debug_assertions)]
use specta_typescript::Typescript;
use tauri::Manager;
use tauri_specta::Builder;

mod boot;
mod collect;
mod commands;
mod config;
mod db;
mod devices;
mod error;
mod events;
mod library;
mod model;
mod pricing;
mod provider;
mod proxy;
mod sessions;
mod source_parser;
mod sync;
mod synced_doc;
mod time;
mod window_geom;

use commands::AppState;

// `set_language` 在语言切换时重建托盘菜单的既有调用路径（commands 侧按
// `crate::tray_menu_for` 引用；构建与签名都在 `boot::tray`）。
pub(crate) use boot::tray_menu_for;

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
        commands::query_distinct_projects,
        commands::query_project_usage,
        commands::query_session_usage,
        commands::query_device_usage,
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
        commands::set_lightweight_auto_tuck,
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
        commands::query_project_stats_cmd,
        commands::query_session_stats_cmd,
        commands::get_session_cmd,
        commands::get_session_transcript_cmd,
        commands::set_session_favorited_cmd,
        commands::set_session_custom_title_cmd,
        commands::set_session_local_group_cmd,
        commands::set_session_synced_group_cmd,
        commands::delete_sessions_cmd,
        commands::create_local_group_cmd,
        commands::rename_local_group_cmd,
        commands::delete_local_group_cmd,
        commands::reorder_local_groups_cmd,
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
        commands::format_toml_cmd,
        commands::extract_snippet_from_live_cmd,
        commands::export_providers_cmd,
        commands::import_providers_cmd,
        commands::fetch_models_cmd,
        commands::add_provider_to_live_cmd,
        commands::remove_provider_from_live_cmd,
        commands::import_providers_from_live_cmd,
        commands::preview_live_import_cmd,
        commands::import_from_ccswitch_cmd,
        window_geom::dock_window_right,
        window_geom::center_window,
        window_geom::set_window_rect,
    ])
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

/// Process bootstrap: dev binding export, plugin / event-hook wiring, then the
/// boot step list ([`boot::run_boot`]) — the sequence and its failure
/// semantics live in `boot`. Exit-side work hangs off the hooks here:
/// close→tray routing in `on_window_event`, the pre-quit Artifact flush in
/// `boot::on_run_event`.
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
                        crate::events::emit_close_requested(window.app_handle());
                    }
                }
            }
        })
        .setup(|app| {
            // 完整启动 = boot 清单（次序与失败语义见 boot 模块文档）：Fatal
            // 步骤失败在此上抛 → build 处的 expect 崩溃退出（拆分前 config/
            // store 是裸 expect、window/tray 是 `?`，同样起不来）；BestEffort
            // 失败聚合进 BootReport，不挡启动。
            boot::run_boot(app.handle().clone())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(boot::on_run_event);
}
