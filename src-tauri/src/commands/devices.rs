//! 设备域：本机/对端命名、遗忘对端、设备列表。

use tauri::State;

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::model::DeviceInfo;

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
