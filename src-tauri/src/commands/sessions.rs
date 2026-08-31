//! 会话域：转录查询、收藏/标题/分组归属、本地与同步分组 CRUD。

use tauri::State;

use super::{run_blocking, write_and_emit, AppState, Emit};
use crate::error::AppResult;
use crate::model::{
    GroupTrack, LocalGroup, ProjectStatsRow, SessionFilter, SessionGroup, SessionGroupCounts,
    SessionKey, SessionMessage, SessionQuery, SessionRow, SessionStatsRow, SyncedGroup,
};
use crate::sessions;

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
    track: GroupTrack,
) -> AppResult<SessionGroupCounts> {
    state.store.count_sessions(filter.as_ref(), track)
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

/// The stats dimension at session grain: every session under the filter with
/// its usage four-buckets / hit rate / cost, message count, and per-model
/// token split (see `Store::query_session_stats`). The sessions workbench's
/// left tree and right stats rail consume this one read.
#[tauri::command]
#[specta::specta]
pub fn query_session_stats_cmd(
    state: State<'_, AppState>,
    filter: Option<SessionFilter>,
) -> AppResult<Vec<SessionStatsRow>> {
    state.store.query_session_stats(filter.as_ref())
}

/// One session row by its exact composite key `(id, device_id)` — the
/// "request log → session" jump channel. The frontend resolves a usage
/// record's `session_id` into a title + a jump target through this read (one
/// RTK Query cache row shared by the link and the landing consumer), instead
/// of the usage log query joining session titles backend-side. `None` = no
/// such session (e.g. session-less historical usage).
#[tauri::command]
#[specta::specta]
pub fn get_session_cmd(
    state: State<'_, AppState>,
    id: String,
    device_id: String,
) -> AppResult<Option<SessionRow>> {
    state.store.get_session(&id, &device_id)
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
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.set_session_favorited(&device_id, &id, favorited)
    })
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
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.set_session_custom_title(&device_id, &id, title.as_deref())
    })
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
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.set_session_local_group(&device_id, &id, group_id.as_deref())
    })
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
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.set_session_synced_group(&device_id, &id, group_id.as_deref())
    })
}

/// Batch soft-delete the given sessions (`Store::delete_sessions`): sets the
/// device-private `excluded` marker so the sessions workbench stops surfacing
/// them — source files are never touched, and neither a re-collect nor a
/// peer-snapshot pull can resurrect the marker (it rides no upsert conflict
/// clause). Returns how many rows matched; the confirm step lives frontend-side.
#[tauri::command]
#[specta::specta]
pub fn delete_sessions_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    keys: Vec<SessionKey>,
) -> AppResult<u32> {
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.delete_sessions(&keys).map(|n| n as u32)
    })
}

// ---- local groups ----

#[tauri::command]
#[specta::specta]
pub fn create_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
) -> AppResult<LocalGroup> {
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        let id = sessions::generate_local_group_id();
        let created_at = crate::time::now_iso();
        store.create_local_group(&id, name.trim(), &created_at)
    })
}

#[tauri::command]
#[specta::specta]
pub fn rename_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
    name: String,
) -> AppResult<()> {
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.rename_local_group(&id, name.trim())
    })
}

#[tauri::command]
#[specta::specta]
pub fn delete_local_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> AppResult<()> {
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.delete_local_group(&id)
    })
}

#[tauri::command]
#[specta::specta]
pub fn reorder_local_groups_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    write_and_emit(&state.store, Emit::Sessions(&app_handle), |store| {
        store.reorder_local_groups(&ordered_ids)
    })
}

// ---- synced groups ----

#[tauri::command]
#[specta::specta]
pub async fn create_synced_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    name: String,
) -> AppResult<SyncedGroup> {
    let config = state.config.clone();
    run_blocking(
        "create_synced_group",
        Emit::Sessions(&app_handle),
        move || {
            let cfg = config.get();
            let paths = config.paths();
            sessions::create_synced_group_owned(&paths, &cfg, &name)
        },
    )
    .await
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
    run_blocking(
        "rename_synced_group",
        Emit::Sessions(&app_handle),
        move || {
            let cfg = config.get();
            let paths = config.paths();
            sessions::rename_synced_group_owned(&paths, &cfg, &id, &name)
        },
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_synced_group_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> AppResult<()> {
    let config = state.config.clone();
    run_blocking(
        "delete_synced_group",
        Emit::Sessions(&app_handle),
        move || {
            let cfg = config.get();
            let paths = config.paths();
            sessions::delete_synced_group_owned(&paths, &cfg, &id)
        },
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn reorder_synced_groups_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    let config = state.config.clone();
    run_blocking(
        "reorder_synced_group",
        Emit::Sessions(&app_handle),
        move || {
            let cfg = config.get();
            let paths = config.paths();
            sessions::reorder_synced_groups_owned(&paths, &cfg, &ordered_ids)
        },
    )
    .await
}

/// Unified groups list (local + synced) for one-shot UI fetch.
#[tauri::command]
#[specta::specta]
pub fn list_groups_cmd(state: State<'_, AppState>) -> AppResult<Vec<SessionGroup>> {
    sessions::list_groups_dto(&state.store, &state.config.paths())
}
