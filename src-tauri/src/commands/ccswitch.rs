//! 从 CC-Switch 导入供应商：定位本机配置（SQLite / 旧 JSON）→ 转换 → 复用
//! `provider::import` 的 store 层 seam 写库。

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use tauri::State;

use super::{run_blocking, AppState, Emit};
use crate::error::{AppError, AppResult};
use crate::model::App;
use crate::provider::import::{ImportKeyStrategy, ProviderImportMode};
use crate::provider::import_ccswitch;

/// CC-Switch 配置回退候选（纯函数，测试直测）：默认 `~/.cc-switch/cc-switch.db`
/// →（Windows legacy）`$HOME/.cc-switch/cc-switch.db` → 旧版
/// `~/.cc-switch/config.json`，返回顺序即优先级（逐个 `exists()` 命中即选）。
/// `home` 是注入的 home 目录；`legacy_home` 是 Windows 上从 HOME 环境变量解析
/// 出的 legacy 候选根（v3.10.3 误用 HOME 环境变量的旧版本兼容分支，仅
/// Windows；非 Windows / 未设置 HOME → None）。
fn ccswitch_fallback_candidates(home: &Path, legacy_home: Option<&Path>) -> Vec<PathBuf> {
    let cs_dir = home.join(".cc-switch");
    let mut candidates = vec![cs_dir.join("cc-switch.db")];
    if let Some(lh) = legacy_home {
        candidates.push(lh.join(".cc-switch").join("cc-switch.db"));
    }
    candidates.push(cs_dir.join("config.json"));
    candidates
}

/// Windows legacy 兼容候选根：HOME 环境变量存在时返回其值（v3.10.3 误用 HOME
/// 的旧版本，仅 Windows）。未设置 → None（走默认候选）。
#[cfg(windows)]
fn home_env_legacy_root() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// 定位本机 CC-Switch 配置：`custom` 优先；否则在注入的 `home`（+ Windows
/// legacy 候选根）下按 [`ccswitch_fallback_candidates`] 回退。任一存在即用。
/// 都找不到 → 明确错误（前端友好提示）。
fn locate_ccswitch_config(
    custom: &Option<String>,
    home: &Path,
    legacy_home: Option<&Path>,
) -> AppResult<PathBuf> {
    if let Some(p) = custom {
        let path = PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
        return Err(AppError::Config(format!(
            "指定的 CC-Switch 配置位置不存在: {p}"
        )));
    }
    for candidate in ccswitch_fallback_candidates(home, legacy_home) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(AppError::Config(
        "未检测到本机 CC-Switch 配置，请确认 CC-Switch 已安装，或手动指定配置位置".into(),
    ))
}

/// 读 CC-Switch 配置（SQLite 或旧 JSON）→ (独立供应商, 统一供应商)。SQLite 以只读
/// 连接打开（不加写锁，避免干扰可能正在运行的 CC-Switch），读后连接析构关闭。
fn read_ccswitch_source(
    custom: &Option<String>,
    home: &Path,
    legacy_home: Option<&Path>,
) -> AppResult<(
    Vec<import_ccswitch::CcSwitchProvider>,
    Vec<import_ccswitch::UniversalProvider>,
)> {
    let path = locate_ccswitch_config(custom, home, legacy_home)?;
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
    run_blocking(
        "import_from_ccswitch",
        Emit::Providers(&app_handle),
        move || -> AppResult<import_ccswitch::CcSwitchImportReport> {
            let now = crate::time::now_iso();
            let home =
                dirs::home_dir().ok_or_else(|| AppError::Config("无法定位 home 目录".into()))?;
            #[cfg(windows)]
            let legacy_home = home_env_legacy_root();
            #[cfg(not(windows))]
            let legacy_home: Option<PathBuf> = None;
            let (providers, universals) =
                read_ccswitch_source(&db_path, &home, legacy_home.as_deref())?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 回退候选顺序（纯函数）：默认 db →（Windows）legacy db → 旧 JSON。
    #[test]
    fn fallback_candidates_order() {
        let home = tempfile::tempdir().unwrap();
        let legacy = tempfile::tempdir().unwrap();
        assert_eq!(
            ccswitch_fallback_candidates(home.path(), Some(legacy.path())),
            vec![
                home.path().join(".cc-switch").join("cc-switch.db"),
                legacy.path().join(".cc-switch").join("cc-switch.db"),
                home.path().join(".cc-switch").join("config.json"),
            ]
        );
        // 无 legacy 候选根 → 只有两级回退。
        let plain = ccswitch_fallback_candidates(home.path(), None);
        assert_eq!(plain.len(), 2);
        assert_eq!(
            plain[0],
            home.path().join(".cc-switch").join("cc-switch.db")
        );
        assert_eq!(plain[1], home.path().join(".cc-switch").join("config.json"));
    }

    /// custom 显式指定优先于 home 下的任何候选。
    #[test]
    fn locate_prefers_custom_over_fallbacks() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join(".cc-switch")).unwrap();
        fs::write(home.join(".cc-switch").join("cc-switch.db"), b"x").unwrap();
        let custom = tmp.path().join("custom.db");
        fs::write(&custom, b"x").unwrap();
        let found =
            locate_ccswitch_config(&Some(custom.to_str().unwrap().to_string()), &home, None)
                .unwrap();
        assert_eq!(found, custom);
    }

    /// custom 指定但不存在 → 明确错误。
    #[test]
    fn locate_custom_missing_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let err = locate_ccswitch_config(
            &Some(tmp.path().join("nope.db").to_str().unwrap().to_string()),
            tmp.path(),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().contains("指定的 CC-Switch 配置位置不存在"));
    }

    /// 四级回退直测：默认 db →（Windows）legacy db → 旧 JSON，按序取第一个存在；
    /// 一个都不存在 → 明确错误。
    #[test]
    fn locate_falls_back_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let cs = home.join(".cc-switch");

        // 只有默认 db → 默认 db。
        fs::create_dir_all(&cs).unwrap();
        fs::write(cs.join("cc-switch.db"), b"x").unwrap();
        assert_eq!(
            locate_ccswitch_config(&None, &home, None).unwrap(),
            cs.join("cc-switch.db")
        );

        // 默认 db 缺失、legacy db 存在（Windows legacy 分支）→ legacy db。
        let legacy = tmp.path().join("legacy-home");
        fs::create_dir_all(legacy.join(".cc-switch")).unwrap();
        fs::write(legacy.join(".cc-switch").join("cc-switch.db"), b"x").unwrap();
        fs::remove_file(cs.join("cc-switch.db")).unwrap();
        assert_eq!(
            locate_ccswitch_config(&None, &home, Some(&legacy)).unwrap(),
            legacy.join(".cc-switch").join("cc-switch.db")
        );

        // 只剩旧 JSON → 旧 JSON。
        fs::write(cs.join("config.json"), b"{}").unwrap();
        assert_eq!(
            locate_ccswitch_config(&None, &home, None).unwrap(),
            cs.join("config.json")
        );

        // 全部存在 → 默认 db 优先（legacy 不抢先）。
        fs::write(cs.join("cc-switch.db"), b"x").unwrap();
        assert_eq!(
            locate_ccswitch_config(&None, &home, Some(&legacy)).unwrap(),
            cs.join("cc-switch.db")
        );

        // 一个都不存在 → 明确错误。
        fs::remove_file(cs.join("cc-switch.db")).unwrap();
        fs::remove_file(cs.join("config.json")).unwrap();
        let err = locate_ccswitch_config(&None, &home, None).unwrap_err();
        assert!(err.to_string().contains("未检测到本机 CC-Switch 配置"));
    }

    /// read_ccswitch_source 按扩展名路由：旧 JSON 走 parse_legacy_json。
    #[test]
    fn read_source_routes_legacy_json() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join(".cc-switch")).unwrap();
        fs::write(
            home.join(".cc-switch").join("config.json"),
            r#"{"apps":{"claude":{"providers":{}}}}"#,
        )
        .unwrap();
        let (providers, universals) = read_ccswitch_source(&None, &home, None).unwrap();
        assert!(providers.is_empty());
        assert!(universals.is_empty());
    }
}
