//! Dashboard 用量读命令（stats / trend / logs / models / distinct）。

use tauri::State;

use super::AppState;
use crate::error::AppResult;
use crate::model::{
    LogsQuery, ModelStatsRow, ProjectCandidates, TrendBucket, TrendPoint, UsageFilter, UsageLogRow,
    UsageStats,
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
    state.store.query_distinct("source", &filter)
}

#[tauri::command]
#[specta::specta]
pub fn query_distinct_models(
    state: State<'_, AppState>,
    filter: UsageFilter,
) -> AppResult<Vec<String>> {
    state.store.query_distinct("model", &filter)
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
