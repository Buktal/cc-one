//! 手动「采集 / 同步」触发——`collect::align` 的命令层薄壳。
//! The ingest path (`collect_into`) and the manual orchestrators (`align`,
//! `run_sync_round` + postures) live in `collect`; the items here are the typed
//! Tauri commands that bind them.

use tauri::State;

use super::{run_blocking, AppState, Emit};
use crate::collect::AlignReport;
use crate::error::AppResult;

/// The shared command body of `collect_now` and `sync_now` — one manual
///「采集 / 同步」action (`collect::align`) with two frontend triggers (the
/// dashboard button and the Settings「立即同步」entry). Standalone ⇒ collect
/// only; Synced ⇒ collect + sync with retry. Heavy disk/git work → the
/// blocking pool; on success the `usage_changed` invalidation rides the same
/// `run_blocking` call, so the emit cannot drift from the write.
async fn run_manual_align(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<AlignReport> {
    let store = state.store.clone();
    let config = state.config.clone();
    run_blocking("align", Emit::Usage(&app_handle), move || {
        Ok(crate::collect::align(&store, &config))
    })
    .await
}

/// Manual「采集 / 同步」from the dashboard button: collect now, then (Synced
/// only) pull + push with a bounded retry. The run mode decides what it means,
/// not the UI.
#[tauri::command]
#[specta::specta]
pub async fn collect_now(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<AlignReport> {
    run_manual_align(state, app_handle).await
}

/// Manual「立即同步」from the Settings entry — the same action as the dashboard
/// button. Kept as a distinct command so the Settings card has its own trigger
/// next to the repo binding; the work is identical.
#[tauri::command]
#[specta::specta]
pub async fn sync_now(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> AppResult<AlignReport> {
    run_manual_align(state, app_handle).await
}
