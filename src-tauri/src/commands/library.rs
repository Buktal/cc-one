//! 资料库域：扫描、上传、导出、删除、重命名、文本预览、设备子树摘要。
//! 全部命令都是 [`run_blocking`] 薄壳（扫描 / 拷贝 / 删除都是重 IO，主线程
//! 不碰）；资料库域不发事件总线失效信号——前端在 mutation 成功处自己失效
//! Library tag。

use std::path::Path;

use tauri::State;

use super::{run_blocking, AppState, Emit};
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
    run_blocking("library_scan", Emit::None, move || {
        library::scan(&store, &config, &device_scope, &subpath)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn upload_to_library(
    state: State<'_, AppState>,
    items: Vec<UploadItem>,
    subpath: String,
) -> AppResult<()> {
    let config = state.config.clone();
    run_blocking("library_upload", Emit::None, move || {
        let cfg = config.get();
        let paths = config.paths();
        // The domain entry commits + pushes itself — the command stays a thin
        // typed shell over it.
        library::upload(&paths, &cfg, &items, &subpath)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn export_from_library(
    state: State<'_, AppState>,
    rel_path: String,
    target_dir: String,
) -> AppResult<()> {
    let config = state.config.clone();
    run_blocking("library_export", Emit::None, move || {
        let paths = config.paths();
        library::export_entry(&paths, &rel_path, &target_dir)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_from_library(state: State<'_, AppState>, rel_path: String) -> AppResult<()> {
    let config = state.config.clone();
    run_blocking("library_delete", Emit::None, move || {
        let cfg = config.get();
        let paths = config.paths();
        library::delete_entry(&paths, &cfg, &rel_path)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn rename_in_library(
    state: State<'_, AppState>,
    rel_path: String,
    new_name: String,
) -> AppResult<()> {
    let config = state.config.clone();
    run_blocking("library_rename", Emit::None, move || {
        let cfg = config.get();
        let paths = config.paths();
        library::rename_entry(&paths, &cfg, &rel_path, &new_name)
    })
    .await
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
    run_blocking("library_text_read", Emit::None, move || {
        let paths = config.paths();
        library::read_text_entry(&paths, &rel_path)
    })
    .await
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
    run_blocking("library_summary", Emit::None, move || {
        // device_id 是前端直通的命令参数，直接 `join` 进 library root——先过
        // 词法包含谓词（与 library 域其它路径参数同一份，见
        // `library::device_subdir`）：`..` 原地穿越、绝对前缀整体替换 base，
        // 都必须在 join 之前拒绝，读向不越出 library root。
        if !library::has_only_plain_components(Path::new(&device_id)) {
            return Err(AppError::Config(format!(
                "library path escapes the root: {device_id}"
            )));
        }
        let paths = config.paths();
        Ok(library::count_subtree(&paths.library.join(&device_id)))
    })
    .await
}
