//! 会话域：转录查询、收藏/标题/分组归属、本地与同步分组 CRUD。

use tauri::{Emitter, State};

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::model::{
    LocalGroup, ProjectStatsRow, SessionFilter, SessionGroup, SessionGroupCounts, SessionMessage,
    SessionQuery, SessionRow, SyncedGroup,
};
use crate::sessions;

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

/// The project dimension: sessions rolled up by project identity with their
/// usage aggregates (session count / requests / token four-buckets / cache-hit
/// rate / cost / last active). Worktree sessions land under their parent
/// project. The filter applies before grouping (time range etc. narrow which
/// sessions feed the buckets).
#[tauri::command]
#[specta::specta]
pub fn query_project_stats_cmd(
    state: State<'_, AppState>,
    filter: Option<SessionFilter>,
) -> AppResult<Vec<ProjectStatsRow>> {
    state.store.query_project_stats(filter.as_ref())
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
