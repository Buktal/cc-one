//! Sync repo binding + probe（Settings「同步」卡片的仓库绑定域）。

use tauri::State;

use super::{run_blocking, AppState, Emit};
use crate::error::AppResult;
use crate::model::RunMode;
use crate::sync::VerifyReport;

/// Configure the sync repo + PAT, upgrading Standalone → Synced, then
/// immediately run one sync round (pull peers + push self) so peer devices show
/// up and this device's existing data reaches the repo without a restart (the
/// startup sync only fires on next launch). The round comes from
/// `collect::run_sync_round` under the
/// [`collect::SyncRoundPosture::OnceLogged`] posture — the same round the
/// scheduler runs each push interval, but WITHOUT the retry wrapping that
/// `align` (the manual collect/sync buttons) applies: a failure here is logged
/// by the posture and left for the next startup sync to retry, not retried in
/// place. Best-effort: a sync failure doesn't undo the bind.
#[tauri::command]
#[specta::specta]
pub async fn set_sync_repo(
    state: State<'_, AppState>,
    repo_url: String,
    github_token: String,
) -> AppResult<RunMode> {
    let config = state.config.clone();
    let store = state.store.clone();
    run_blocking(
        "set_sync_repo",
        Emit::None,
        move || -> AppResult<RunMode> {
            let cfg = config.update(|c| {
                c.repo_url = if repo_url.trim().is_empty() {
                    None
                } else {
                    Some(repo_url.trim().to_string())
                };
                c.github_token = if github_token.trim().is_empty() {
                    None
                } else {
                    Some(github_token.trim().to_string())
                };
            })?;
            if cfg.is_synced() {
                // 绑定后立刻跑一轮 pull+push（只跑一轮、outcome 就地记日志）；
                // 失败留给下次启动同步重试，不在原地重试。
                crate::collect::run_sync_round(
                    &store,
                    &config,
                    crate::collect::SyncRoundPosture::OnceLogged("set_sync_repo"),
                );
            }
            Ok(cfg.mode())
        },
    )
    .await
}

/// Unbind the repo, downgrading to Standalone. Clears the local
/// `.git` so a re-bind (often to a different repo) starts clean instead of
/// reusing the old remote/branch. Usage rows (DB) and `data/` are retained.
/// `reset_local_git` 对仓库 `remove_dir_all(.git)`，大仓库能跑上秒——必须
/// 离开主线程，走 [`run_blocking`]。
#[tauri::command]
#[specta::specta]
pub async fn clear_sync_repo(state: State<'_, AppState>) -> AppResult<RunMode> {
    let config = state.config.clone();
    run_blocking("clear_sync_repo", Emit::None, move || {
        let cfg = config.update(|c| {
            c.repo_url = None;
            c.github_token = None;
        })?;
        let paths = config.paths();
        crate::sync::reset_local_git(&paths.repo);
        Ok(cfg.mode())
    })
    .await
}

/// Probe a sync repo + PAT for reachability (「测试连接」). Pass explicit
/// values to validate BEFORE binding, or `None`/`None` to re-check the already-
/// configured repo. Pure ls-remote — never mutates config or the real sync repo.
/// Always returns `Ok(report)`; the probe's own outcome (auth ok / bad token /
/// not found) lives in `report.ok`, so the frontend never throws on a failed
/// probe (only a `spawn_blocking` join failure surfaces as an `AppError`).
#[tauri::command]
#[specta::specta]
pub async fn verify_sync_repo(
    state: State<'_, AppState>,
    repo_url: Option<String>,
    github_token: Option<String>,
) -> AppResult<VerifyReport> {
    let config = state.config.clone();
    run_blocking(
        "verify_sync_repo",
        Emit::None,
        move || -> AppResult<VerifyReport> {
            let cfg = config.get();
            let report = match (repo_url, github_token) {
                // Validate an as-yet-unbound pair straight from the Settings inputs.
                (Some(url), Some(tok)) => crate::sync::verify_remote(&url, &tok),
                // Re-check the configured repo: the raw PAT never crosses to JS, so
                // the masked_token the UI shows can't drive a re-probe — read the
                // real token server-side from config.
                (None, None) => match (cfg.repo_url.as_deref(), cfg.github_token.as_deref()) {
                    (Some(url), Some(tok)) => crate::sync::verify_remote(url, tok),
                    _ => crate::sync::verify_remote("", ""),
                },
                // One field present, the other absent: surface as an input error.
                _ => crate::sync::verify_remote("", ""),
            };
            Ok(report)
        },
    )
    .await
}
