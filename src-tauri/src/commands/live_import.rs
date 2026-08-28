//! live 配置文件反向导入域：读盘命令薄壳。「读 live → 0..N 条待导入条目」的
//! seam（multiplicity 进接口：附加模式 N 条共存、单激活 0..1 条）与预览 /
//! 导入的同源推导在 `provider::import_live` + `live_adapter`；附加模式「加入 /
//! 移出 live」的编排两半都在 `provider::activation`。本模块只剩路径解析、读
//! 盘、wire 形状与 emit。

use std::collections::HashMap;
use std::path::PathBuf;

use tauri::State;

use super::{emit_providers_changed, AppState};
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::{activation, import_live, live};

/// 附加模式「添加」按钮：把 provider ensure-in-live（写进 opencode.json + 设
/// liveManaged=true）。仅附加模式 app 有意义（单激活用 switch_provider_cmd）。
#[tauri::command]
#[specta::specta]
pub async fn add_provider_to_live_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<Provider> {
    let store = state.store.clone();
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<Provider> {
        let provider = store
            .get_provider(app, &id)?
            .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
        // 编排与 switch 的附加分支共用 provider::activation（单一归属）。
        let paths = activation::resolve_paths(app)?;
        activation::ensure_opencode_in_live(&store, provider, &paths.opencode_config)
    })
    .await
    .map_err(|e| AppError::Internal(format!("add_provider_to_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 附加模式「移除」按钮：从 live 配置删 provider 条目（DB 记录保留，随时再加
/// 回来）。编排（撤除写盘 + meta.liveManaged=false + 落库，liveKey 保留）收口
/// 在 [`activation::remove_from_live`]——与删除供应商路径共用同一入口。
#[tauri::command]
#[specta::specta]
pub async fn remove_provider_from_live_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<Provider> {
    let store = state.store.clone();
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<Provider> {
        let provider = store
            .get_provider(app, &id)?
            .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
        let paths = activation::resolve_paths(app)?;
        activation::remove_from_live(&store, provider, &paths.opencode_config)
    })
    .await
    .map_err(|e| AppError::Internal(format!("remove_provider_from_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 「从 live 配置导入」：读该应用的 live 文件(s)（路径序 = [`App::live_paths`]，
/// 附加模式即 opencode.json）→ 0..N 条待导入条目 → 落库。条目推导与冲突键
/// 策略（单激活 Name / 附加模式 LiveKey，按 `is_additive_mode` 分派）都在
/// [`import_live::import_from_live_texts`]。返回写入条数。
#[tauri::command]
#[specta::specta]
pub async fn import_providers_from_live_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    name_overrides: HashMap<String, String>,
) -> AppResult<u32> {
    let store = state.store.clone();
    let count = tauri::async_runtime::spawn_blocking(move || -> AppResult<u32> {
        let texts = live::read_app_live_texts(app)?;
        import_live::import_from_live_texts(&store, app, &texts, &name_overrides)
    })
    .await
    .map_err(|e| AppError::Internal(format!("import_providers_from_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(count)
}

// ---------------- 「从 live 配置导入」预览（只读）----------------

/// 「从 live 配置导入」的预览载荷（附加模式与单激活应用共用，ADR-0012）。
/// 附加模式的配置文件不存在 → `Missing`（带完整路径，前端展示）；存在 →
/// 将导入的条目列表（空 = 无 provider 段 / 无可导入）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LiveImportPreview {
    Missing {
        path: String,
    },
    Ready {
        entries: Vec<LiveImportPreviewEntry>,
    },
}

/// 一条将导入的供应商预览。**密钥绝不进预览载荷**——只有布尔
/// `has_secret`，apiKey / headers 值不跨边界（泄漏守卫见
/// `preview_payload_never_contains_secret_value` 测试）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LiveImportPreviewEntry {
    /// `provider.<key>`（附加模式）或 name（单激活应用），即导入后的去重键。
    pub key: String,
    /// 显示名（entry.name 优先 / base_url 注册域 / key 回退——推导在
    /// `import_live`，与导入共用同一份）。
    pub name: String,
    /// 名字是否由 base_url 的注册域推导（单激活应用）；附加模式的名字来自
    /// entry.name / key，恒 false。前端理由行只在该标志为 true 时显示
    /// 「名取自 <url>」——否则对用户说谎。
    pub name_derived_from_url: bool,
    /// options.baseURL（附加模式）或 live 里 base_url（单激活），缺 → ""。
    pub base_url: String,
    /// options.apiKey / options.headers（附加模式）或 settingsConfig 携带凭据
    /// （单激活）任一非空。
    pub has_secret: bool,
    /// DB 无此去重键 → 新建；有 → 更新（与导入的冲突键判定一致）。
    pub is_new: bool,
    /// 可共享键候选（单激活应用导入后可提取为通用片段；附加模式无此概念 →
    /// 空）。
    pub snippet_candidates: Vec<String>,
}

impl From<import_live::LiveImportPreviewRow> for LiveImportPreviewEntry {
    fn from(row: import_live::LiveImportPreviewRow) -> Self {
        LiveImportPreviewEntry {
            key: row.entry.key,
            name: row.entry.name,
            name_derived_from_url: row.entry.name_derived_from_url,
            base_url: row.entry.base_url,
            has_secret: row.entry.has_secret,
            is_new: row.is_new,
            snippet_candidates: row.entry.snippet_candidates,
        }
    }
}

/// 预览核心（路径注入，测试可换临时目录）：读 live 文本 → 域侧预览行 →
/// wire 形状。附加模式的配置文件不存在 → `Missing`（配置文件是附加模式唯一
/// 事实源，缺席值得明示）；单激活 live 缺席 = 从未配置，Ready 空即可。
fn preview_live_import_at(
    store: &Store,
    app: App,
    paths: &[PathBuf],
) -> AppResult<LiveImportPreview> {
    if app.is_additive_mode() {
        if let Some(path) = paths.first() {
            if !path.exists() {
                return Ok(LiveImportPreview::Missing {
                    path: path.display().to_string(),
                });
            }
        }
    }
    let mut texts = Vec::with_capacity(paths.len());
    for path in paths {
        texts.push(live::read_live_settings(path)?);
    }
    let rows = import_live::preview_from_live_texts(store, app, &texts)?;
    Ok(LiveImportPreview::Ready {
        entries: rows.into_iter().map(LiveImportPreviewEntry::from).collect(),
    })
}

/// 「从本机配置文件导入」预览（只读命令）：返回将导入的供应商（名称/端点/
/// 是否含密钥/新建或更新/片段候选）。确认导入仍走 import_providers_from_live_cmd。
/// 不 emit、不失效任何 tag。
#[tauri::command]
#[specta::specta]
pub async fn preview_live_import_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<LiveImportPreview> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let paths = app.live_paths()?;
        preview_live_import_at(&store, app, &paths)
    })
    .await
    .map_err(|e| AppError::Internal(format!("preview_live_import task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::mem;
    use crate::provider::import_live::tests::opencode_live_json;

    /// 预览壳：附加模式配置文件不存在 → Missing 变体（带完整路径），不是 Err。
    #[test]
    fn preview_shell_missing_file_is_missing_variant_for_additive_mode() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        match preview_live_import_at(&s, App::OpenCode, std::slice::from_ref(&path))
            .expect("missing is Ok")
        {
            LiveImportPreview::Missing { path: shown } => {
                assert_eq!(shown, path.display().to_string());
            }
            LiveImportPreview::Ready { .. } => panic!("缺文件应 Missing"),
        }
    }

    /// 预览壳：附加模式文件存在 → Ready + 条目（经真实读盘路径）。
    #[test]
    fn preview_shell_ready_with_seeded_file() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        std::fs::write(&path, opencode_live_json()).expect("write live file");
        match preview_live_import_at(&s, App::OpenCode, &[path]).expect("ready is Ok") {
            LiveImportPreview::Ready { entries } => {
                assert_eq!(entries.len(), 2);
                assert!(entries.iter().all(|e| e.is_new), "空 DB → 全部新建");
                assert!(
                    entries.iter().all(|e| !e.name_derived_from_url),
                    "附加模式的名字来自 entry.name / key，不由 URL 推导"
                );
            }
            LiveImportPreview::Missing { .. } => panic!("文件存在应 Ready"),
        }
    }

    /// 预览壳：单激活 live 文件缺失 = 从未配置 → Ready 空（无 Missing 态——
    /// Missing 是附加模式配置文件缺席的专属语义）。
    #[test]
    fn preview_shell_single_activate_missing_file_is_ready_empty() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        match preview_live_import_at(&s, App::Claude, &[path]).expect("missing is Ok") {
            LiveImportPreview::Ready { entries } => assert!(entries.is_empty()),
            LiveImportPreview::Missing { .. } => panic!("单激活缺文件应 Ready 空"),
        }
    }

    /// 单激活预览壳贯通（真实读盘路径）：种入 settings.json → 1 条 ready 条目。
    #[test]
    fn preview_shell_single_activate_reads_seeded_file() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_MODEL":"kimi"}}"#,
        )
        .expect("write live file");
        match preview_live_import_at(&s, App::Claude, &[path]).expect("ready is Ok") {
            LiveImportPreview::Ready { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].key, "moonshot");
                assert!(entries[0].name_derived_from_url, "名字取自 base_url");
                assert!(!entries[0].has_secret, "无凭据键 → 不含密钥");
                assert!(entries[0]
                    .snippet_candidates
                    .contains(&"ANTHROPIC_MODEL".to_string()));
            }
            LiveImportPreview::Missing { .. } => panic!("文件存在应 Ready"),
        }
    }

    /// 防泄漏回归护栏：预览载荷序列化后绝不包含密钥值（apiKey 只出布尔）。
    #[test]
    fn preview_payload_never_contains_secret_value() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        std::fs::write(&path, opencode_live_json()).expect("write live file");
        let preview = preview_live_import_at(&s, App::OpenCode, &[path]).expect("ready is Ok");
        let json = serde_json::to_string(&preview).expect("preview serialize");
        assert!(!json.contains("sk-x"), "preview 载荷不得携带密钥值: {json}");
    }
}
