//! Tauri command layer (typed contract — the query boundary).
//!
//! Every command is `#[specta::specta]` with typed args/return/error; tauri-specta
//! generates the matching typed JS function. `tauri::State` args are injected by
//! the runtime and excluded from the JS signature. JS never sees SQL.
//!
//! The state holds `Arc`s so blocking work can be moved onto `spawn_blocking`
//! without borrowing the request-scoped `State` (which is not `'static`).
//!
//! Commands are split by domain into submodules (sync repo binding / devices /
//! collect triggers / usage queries / pricing / preferences / sessions /
//! providers / config snippet / live import / CC-Switch import / library). This
//! module owns the shared `AppState` and re-exports every command, so `lib.rs`
//! registers each as `commands::<name>` regardless of its home submodule — the
//! `#[tauri::command]` hidden handler macros live at the crate root either way.

mod ccswitch;
mod collect;
mod devices;
mod library;
mod live_import;
mod preferences;
mod pricing;
mod providers;
mod sessions;
mod snippet;
mod sync_repo;
mod usage;

pub use ccswitch::*;
pub use collect::*;
pub use devices::*;
pub use library::*;
pub use live_import::*;
pub use preferences::*;
pub use pricing::*;
pub use providers::*;
pub use sessions::*;
pub use snippet::*;
pub use sync_repo::*;
pub use usage::*;

use std::sync::Arc;

use tauri::State;

use crate::config::ConfigStore;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::events;
use crate::model::RunMode;

/// 写后通知轨（执行轨在 [`run_blocking`]）：blocking 工作成功后要发哪条失效
/// 事件。发事件的变体持有 `AppHandle`——要 emit 就必须先把 handle 交出来，
/// 于是「漏发」的形态从「少了一行没人注意的代码」变成「review 一眼可见的
/// `Emit::None`」；事件名与 TS 消费名单的单源见 [`crate::events`]。
#[derive(Clone, Copy)]
pub(crate) enum Emit<'a> {
    /// 只读工作，或写后不经事件总线失效（前端在 mutation 成功处自己失效）。
    None,
    /// Store 整体写（采集 / 同步）→ `usage_changed`。
    Usage(&'a tauri::AppHandle),
    /// 会话域写 → `sessions_changed`。
    Sessions(&'a tauri::AppHandle),
    /// 供应商域写 → `providers_changed`。
    Providers(&'a tauri::AppHandle),
}

/// 命令层重活唯一入口：执行轨（blocking 执行 + join 失败 label 化）与通知轨
/// （emit 配对）都钉在这一处。闭包跑进 blocking 线程池——async runtime 的
/// 线程不做磁盘 / git / 网络 IO，主线程不碰重活（UI 不冻结）；join 失败统一
/// 按 `label` 归因成 `AppError::Internal`；工作成功后才发 `Emit` 声明的失效
/// 事件（失败不发——前端缓存不被半成品写污染）。新写命令不得再手抄
/// `spawn_blocking` + map_err，更不得绕过本入口在别处 emit。
///
/// 明示例外（唯一一处）：config.json 的写盘是同目录 temp+rename 的小文件
/// 原子写（收口在 [`crate::config::ConfigStore`]），属主线程可承担的轻 IO——
/// 偏好类同步命令在主线程直调 `ConfigStore::update` 不算违约。该例外由
/// ConfigStore 单点拥有（config.json 的读写都收在它一处），除它之外的任何
/// 文件写仍是重活，必须走本入口离开主线程。
pub(crate) async fn run_blocking<T, F>(label: &str, emit: Emit<'_>, f: F) -> AppResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    let out = tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Internal(format!("{label} task failed: {e}")))??;
    match emit {
        Emit::None => {}
        Emit::Usage(handle) => events::emit_usage_changed(handle),
        Emit::Sessions(handle) => events::emit_sessions_changed(handle),
        Emit::Providers(handle) => events::emit_providers_changed(handle),
    }
    Ok(out)
}

/// App-wide managed state: the Local Store + local config, wrapped
/// in `Arc` so blocking tasks can take owned clones.
pub struct AppState {
    pub store: Arc<Store>,
    pub config: Arc<ConfigStore>,
}

/// Snapshot of app/status info for the UI on startup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct AppInfo {
    pub device_id: String,
    pub display_name: String,
    pub mode: RunMode,
    pub repo_url: Option<String>,
    pub masked_token: Option<String>,
    pub github_user: Option<String>,
    pub claude_projects_dir: Option<String>,
    pub version: String,
}

/// App status: device, mode (Standalone/Synced), paths, version.
#[tauri::command]
#[specta::specta]
pub fn get_app_info(state: State<'_, AppState>) -> AppResult<AppInfo> {
    let cfg = state.config.get();
    let claude_dir = crate::source_parser::default_projects_dir().map(|p| p.display().to_string());
    Ok(AppInfo {
        device_id: cfg.device_id.clone(),
        display_name: cfg.display_name.clone(),
        mode: cfg.mode(),
        repo_url: cfg.repo_url.clone(),
        masked_token: cfg.masked_token(),
        github_user: cfg.github_user.clone(),
        claude_projects_dir: claude_dir,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}
