//! 资料库域：扫描、上传、导出、删除、重命名、文本预览、设备子树摘要。

use tauri::State;

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::library::{self, DeviceLibrarySummary, LibraryEntry, UploadItem};

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
