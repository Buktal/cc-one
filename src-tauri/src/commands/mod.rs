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
use crate::error::AppResult;
use crate::model::RunMode;

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
