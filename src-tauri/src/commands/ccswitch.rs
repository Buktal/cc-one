//! 从 CC-Switch 导入供应商：定位本机配置（SQLite / 旧 JSON）→ 转换 → 复用
//! `provider::import` 的 store 层 seam 写库。

use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};
use tauri::State;

use super::providers::emit_providers_changed;
use super::AppState;
use crate::error::{AppError, AppResult};
use crate::model::App;
use crate::provider::import::{ImportKeyStrategy, ProviderImportMode};
use crate::provider::import_ccswitch;

/// 定位本机 CC-Switch 配置：`custom` 优先；否则回退顺序 = 默认 `~/.cc-switch/
/// cc-switch.db` →（Windows）legacy `$HOME/.cc-switch/cc-switch.db` → 旧版
/// `~/.cc-switch/config.json`。任一存在即用。都找不到 → 明确错误（前端友好提示）。
fn locate_ccswitch_config(custom: &Option<String>) -> AppResult<PathBuf> {
    if let Some(p) = custom {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
        return Err(AppError::Config(format!(
            "指定的 CC-Switch 配置位置不存在: {p}"
        )));
    }
    let home = dirs::home_dir().ok_or_else(|| AppError::Config("无法定位 home 目录".into()))?;
    let cs_dir = home.join(".cc-switch");
    let db = cs_dir.join("cc-switch.db");
    if db.exists() {
        return Ok(db);
    }
    #[cfg(windows)]
    {
        // 兼容 v3.10.3 误用 HOME 环境变量的旧版本（仅 Windows）。
        if let Ok(h) = std::env::var("HOME") {
            let legacy = PathBuf::from(h).join(".cc-switch").join("cc-switch.db");
            if legacy.exists() {
                return Ok(legacy);
            }
        }
    }
    let json = cs_dir.join("config.json");
    if json.exists() {
        return Ok(json);
    }
    Err(AppError::Config(
        "未检测到本机 CC-Switch 配置，请确认 CC-Switch 已安装，或手动指定配置位置".into(),
    ))
}

/// 读 CC-Switch 配置（SQLite 或旧 JSON）→ (独立供应商, 统一供应商)。SQLite 以只读
/// 连接打开（不加写锁，避免干扰可能正在运行的 CC-Switch），读后连接析构关闭。
fn read_ccswitch_source(
    custom: &Option<String>,
) -> AppResult<(
    Vec<import_ccswitch::CcSwitchProvider>,
    Vec<import_ccswitch::UniversalProvider>,
)> {
    let path = locate_ccswitch_config(custom)?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext == "db" {
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let providers = import_ccswitch::read_providers_from_db(&conn)?;
        let universals = import_ccswitch::read_universals_from_db(&conn)?;
        Ok((providers, universals))
    } else {
        let text = std::fs::read_to_string(&path)?;
        import_ccswitch::parse_legacy_json(&text)
    }
}

/// 「从 CC-Switch 导入」按钮：定位本机 CC-Switch 配置 → 读 + 转换供应商 → 直接
/// 喂 store 层 seam（merge / overwrite）写本机库——不再序列化成导出文档绕道，
/// 冲突键 (app, id)、只写本机 DB 由 seam 守住。**单应用语境**：`app` 是当前
/// 视图的应用，只搬该应用的供应商（claude 视图不冒出 codex 供应商，见
/// ADR-0012）。代理 / OAuth / 不支持应用的供应商跳过并进报告明细。找不到配置
/// → 明确错误（前端展示友好提示）。
#[tauri::command]
#[specta::specta]
pub async fn import_from_ccswitch_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    mode: ProviderImportMode,
    db_path: Option<String>,
) -> AppResult<import_ccswitch::CcSwitchImportReport> {
    let store = state.store.clone();
    let report = tauri::async_runtime::spawn_blocking(
        move || -> AppResult<import_ccswitch::CcSwitchImportReport> {
            let now = crate::time::now_iso();
            let (providers, universals) = read_ccswitch_source(&db_path)?;
            let (imported, skipped) =
                import_ccswitch::collect_ccswitch_imports(&providers, &universals, app, &now);
            let apply = crate::provider::import::import_providers(
                &store,
                &imported,
                ImportKeyStrategy::AppId(mode),
            )?;
            Ok(import_ccswitch::CcSwitchImportReport {
                imported: apply.imported,
                merge_skipped: apply.skipped,
                proxy_skipped: skipped,
            })
        },
    )
    .await
    .map_err(|e| AppError::Internal(format!("import_from_ccswitch task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(report)
}
