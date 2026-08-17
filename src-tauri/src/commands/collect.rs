//! 手动「采集 / 同步」触发——`collect::align` 的命令层薄壳。
//! The ingest path (`collect_into`) and the manual orchestrators (`align`,
//! `sync_round`) live in `collect`; the items here are the typed Tauri commands
//! that drive them.

use tauri::{Emitter, State};

use super::AppState;
use crate::collect::AlignReport;
use crate::error::{AppError, AppResult};

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
