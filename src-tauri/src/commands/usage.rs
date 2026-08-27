//! Dashboard 用量读命令（stats / trend / logs / models / distinct）。

use tauri::State;

use super::AppState;
use crate::db::DistinctColumn;
use crate::error::AppResult;
use crate::model::{
    DeviceUsageRow, LogsQuery, ModelStatsRow, ProjectCandidates, ProjectUsageRow, SessionUsageRow,
    TrendBucket, TrendPoint, UsageFilter, UsageLogRow, UsageStats,
};

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
    state.store.query_distinct(DistinctColumn::Source, &filter)
}

#[tauri::command]
#[specta::specta]
pub fn query_distinct_models(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<String>> {
    state.store.query_distinct(DistinctColumn::Model, &filter)
}

/// Distinct project candidates for the project dropdown (facet semantics —
/// the filter's own project value is ignored). Known projects come from the
/// sessions-side registry (本机全部会话 ∪ 远程收藏快照); the unknown-project
/// sentinel rides as data in `unknown` when session-less usage exists in the
/// window, so the frontend labels the special option without a second copy of
/// the literal.
#[tauri::command]
#[specta::specta]
pub fn query_distinct_projects(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<ProjectCandidates> {
    state.store.query_distinct_projects(&filter)
}

/// Project buckets at usage grain (#106 dashboard project section) — bucket
/// sums equal `query_usage_stats`'s totals under the same filter exactly.
#[tauri::command]
#[specta::specta]
pub fn query_project_usage(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<ProjectUsageRow>> {
    state.store.query_project_usage(&filter)
}

/// Session buckets at usage grain (#106 dashboard session section) — every
/// store-known session with its in-window usage; per-session turn counts ride
/// along under the turn grain's applicable facets.
#[tauri::command]
#[specta::specta]
pub fn query_session_usage(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<SessionUsageRow>> {
    state.store.query_session_usage(&filter)
}

/// Device buckets at usage grain (#107 dashboard device section) — GROUP BY
/// device_id over the same WHERE builder, so bucket sums equal
/// `query_usage_stats`'s totals under the same filter exactly. Pure usage
/// aggregates: naming / "this machine" identity are the frontend's join with
/// `list_devices`.
#[tauri::command]
#[specta::specta]
pub fn query_device_usage(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<DeviceUsageRow>> {
    state.store.query_device_usage(&filter)
}
