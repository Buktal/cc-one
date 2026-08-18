//! live 配置文件反向导入域：附加模式（OpenCode）写盘/移除/导入、单激活应用
//! 快照导入、「从 live 导入」预览。纯转换与落库逻辑在 `provider::import_live`
//! （快照解析 / opencode 反向导入 / 落库走 `provider::import` seam）与
//! `provider::live_opencode`，本模块是文件 IO 的命令层薄壳。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tauri::State;

use super::providers::emit_providers_changed;
use super::AppState;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::{import_live, live, live_opencode};

// ---------------- 附加模式（OpenCode）写盘命令 ----------------

/// 附加模式核心动作（OpenCode）：把 provider ensure-in-live——写进 opencode.json
/// 同时设 `meta.liveManaged = true` 并落库。key 由 `live_opencode::derive_live_key`
/// 派生（优先沿用 meta.liveKey，改名不重算；首次按 name slugify，空 → 回落 id）。
/// **不取消其它 provider、不碰 active_providers**（附加模式无唯一激活）。返回
/// 更新了 meta 的 provider。
pub(super) fn ensure_opencode_in_live(store: &Store, provider: Provider) -> AppResult<Provider> {
    let path = live_opencode::opencode_config_path()?;
    let live_text = live::read_live_settings(&path)?;
    let key =
        live_opencode::derive_live_key(&provider.name, &provider.id, &provider.meta, &live_text);
    live_opencode::set_opencode_provider(&path, &key, &provider.settings_config)?;
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, true)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式移除（OpenCode）：从 opencode.json 删 `provider.<liveKey>` + 设
/// `meta.liveManaged = false`（保留 liveKey，便于再加回来）。撤除写盘走
/// [`live_opencode::remove_from_live_if_managed`]（与删除供应商路径共用）；
/// 无 liveKey（从未写盘）→ 无操作，原样返回。
fn remove_opencode_from_live(store: &Store, provider: Provider) -> AppResult<Provider> {
    live_opencode::remove_from_live_if_managed(&provider)?;
    let Some(key) = live_opencode::meta_live_key(&provider.meta) else {
        return Ok(provider);
    };
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, false)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式「从配置文件导入」：把 opencode.json 的 `provider.<key>` 反向导入 DB。
/// 读盘薄壳——核心逻辑在 `import_live::import_opencode_from_live_text`（可测，
/// 不碰文件系统）。
fn import_opencode_from_live(
    store: &Store,
    app: App,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let path = live_opencode::opencode_config_path()?;
    let live_text = live::read_live_settings(&path)?;
    import_live::import_opencode_from_live_text(store, app, &live_text, name_overrides)
}

/// 单激活应用「从 live 配置文件导入」（ADR-0012 泛化）：读该应用的 live 文件(s)
/// 反向解析为快照（至多 1 个），经 `import_live::import_snapshot` 落库（按
/// name 去重，走导入 seam）。opencode 走 [`import_opencode_from_live`]。返回
/// 写入条数。`name_overrides` = 预览列表里用户行内改过的名字（key → name；
/// 单激活 key == name）。
fn import_single_activate_from_live(
    store: &Store,
    app: App,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let Some(snap) = read_live_snapshot(app)? else {
        return Ok(0);
    };
    let mut snap = snap;
    if let Some(name) = name_overrides.get(&snap.name) {
        snap.name = name.clone();
    }
    import_live::import_snapshot(store, app, &snap)
}

/// 按 app 读 live 文件文本：路径映射收口在 `live_adapter` 的
/// [`App::live_paths`]（单一事实来源——写盘 / 快照 / 片段提取共用）。
/// opencode 无单份 live 配置概念 → `None`。
pub(super) fn read_live_texts(app: App) -> AppResult<Option<Vec<String>>> {
    let Some(paths) = app.live_paths()? else {
        return Ok(None);
    };
    let mut texts = Vec::with_capacity(paths.len());
    for p in paths {
        texts.push(live::read_live_settings(&p)?);
    }
    Ok(Some(texts))
}

/// 按 app 读 live 文件(s) + 反向解析为快照（分派在 `live_adapter` 的
/// [`App::snapshot_from_texts`]）。opencode → `unreachable`（走 opencode
/// 专属路径）。
fn read_live_snapshot(app: App) -> AppResult<Option<import_live::LiveImportSnapshot>> {
    let Some(texts) = read_live_texts(app)? else {
        return Ok(None);
    };
    Ok(app.snapshot_from_texts(&texts))
}

/// 单激活应用「从 live 导入」预览：快照 → 0/1 条 entry（key = name，is_new 按
/// name 是否已存在）。文件缺失/无可导入 → Ready 空（与 opencode 的空 provider
/// 段同语义；opencode 的 Missing 状态保留给它的路径不存在场景）。
fn preview_single_activate(store: &Store, app: App) -> AppResult<LiveImportPreview> {
    let existing_names: HashSet<String> = store
        .list_providers_for(app)?
        .into_iter()
        .map(|p| p.name)
        .collect();
    let Some(snap) = read_live_snapshot(app)? else {
        return Ok(LiveImportPreview::Ready { entries: vec![] });
    };
    Ok(LiveImportPreview::Ready {
        entries: vec![LiveImportPreviewEntry {
            key: snap.name.clone(),
            name: snap.name.clone(),
            name_derived_from_url: !snap.base_url.is_empty(),
            base_url: snap.base_url.clone(),
            has_secret: snap.has_secret,
            is_new: !existing_names.contains(&snap.name),
            snippet_candidates: snap.snippet_candidates,
        }],
    })
}

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
        ensure_opencode_in_live(&store, provider)
    })
    .await
    .map_err(|e| AppError::Internal(format!("add_provider_to_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 附加模式「移除」按钮：从 opencode.json 删 provider（设 liveManaged=false，DB
/// 记录保留，随时再加回来）。
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
        remove_opencode_from_live(&store, provider)
    })
    .await
    .map_err(|e| AppError::Internal(format!("remove_provider_from_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 附加模式「从配置文件导入」按钮：把现有 opencode.json 的 `provider.*` 反向拉进
/// cc one DB。返回导入/更新条数。
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
        // opencode 附加模式多条目共存；单激活应用一份 live → 至多一个快照。
        if app == App::OpenCode {
            import_opencode_from_live(&store, app, &name_overrides)
        } else {
            import_single_activate_from_live(&store, app, &name_overrides)
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("import_providers_from_live task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(count)
}

// ---------------- 附加模式（OpenCode）导入预览 ----------------

/// 「从 live 配置导入」的预览载荷（opencode 与单激活应用共用，ADR-0012）。
/// 文件不存在 → `Missing`（带完整路径，前端展示，仅 opencode 路径）；存在 →
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
/// `has_secret`，apiKey / headers 值不跨边界（见 `secret_in_entry`）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LiveImportPreviewEntry {
    /// `provider.<key>`（opencode）或 name（单激活应用），即导入后的去重键。
    pub key: String,
    /// entry.name 非空优先，缺失或空串 → key（与导入共用
    /// `live_opencode::entry_display_name`，同一推导）。
    pub name: String,
    /// 名字是否由 base_url 的注册域推导（单激活应用，后端 host_of）；opencode
    /// 的名字来自 entry.name / key，恒 false。前端理由行只在该标志为 true 时
    /// 显示「名取自 <url>」——否则对用户说谎。
    pub name_derived_from_url: bool,
    /// options.baseURL（opencode）或 live 里 base_url（单激活），缺 → ""。
    pub base_url: String,
    /// options.apiKey / options.headers（opencode）或 settingsConfig 携带凭据
    /// （单激活）任一非空。
    pub has_secret: bool,
    /// DB 无此 key → 新建；有 → 更新（与导入的判定一致）。
    pub is_new: bool,
    /// 可共享键候选（单激活应用导入后可提取为通用片段；opencode 无此概念 → 空）。
    pub snippet_candidates: Vec<String>,
}

/// 预览「从 opencode.json 导入」：读盘薄壳——核心逻辑在
/// [`preview_opencode_import_text`]（可测，不碰文件系统/DB）。文件不存在 →
/// `Missing`（不报错：与导入命令「空文件 → 0 条」同属正常路径）。
fn preview_opencode_import(store: &Store, app: App, path: &Path) -> AppResult<LiveImportPreview> {
    if !path.exists() {
        return Ok(LiveImportPreview::Missing {
            path: path.display().to_string(),
        });
    }
    let live_text = live::read_live_settings(path)?;
    let existing_keys: HashSet<String> = store
        .list_providers_for(app)?
        .iter()
        .filter_map(|p| live_opencode::meta_live_key(&p.meta))
        .collect();
    Ok(LiveImportPreview::Ready {
        entries: preview_opencode_import_text(&live_text, &existing_keys),
    })
}

/// preview 核心逻辑（可测）：复用 [`live_opencode::provider_entries`]（单一事实
/// 来源，不重新解析），把 `provider.<key>` 转成预览条目。existing_keys 由调用
/// 方从 DB 收集——「新建 vs 更新」判定与 import（meta.liveKey 集合）一致。
fn preview_opencode_import_text(
    live_text: &str,
    existing_keys: &HashSet<String>,
) -> Vec<LiveImportPreviewEntry> {
    live_opencode::provider_entries(live_text)
        .into_iter()
        .map(|(key, entry)| LiveImportPreviewEntry {
            key: key.clone(),
            name: live_opencode::entry_display_name(&entry, &key),
            name_derived_from_url: false,
            base_url: entry
                .pointer("/options/baseURL")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            has_secret: secret_in_entry(&entry),
            is_new: !existing_keys.contains(&key),
            snippet_candidates: vec![],
        })
        .collect()
}

/// entry 里是否携带凭据（只出布尔，不回取密钥值）：options.apiKey 非空，或
/// options.headers 任一值非空（headers 可携带 Authorization 等认证头）。
fn secret_in_entry(entry: &serde_json::Value) -> bool {
    if entry
        .pointer("/options/apiKey")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    entry
        .pointer("/options/headers")
        .and_then(|h| h.as_object())
        .is_some_and(|m| {
            m.values()
                .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        })
}

/// 「从本机配置文件导入」预览：只读命令，按 app 分派（opencode 走读盘 +
/// Missing 状态；单激活应用走快照解析），返回将导入的供应商（名称/端点/是否
/// 含密钥/新建或更新/片段候选）。确认导入仍走 import_providers_from_live_cmd。
/// 不 emit、不失效任何 tag。
#[tauri::command]
#[specta::specta]
pub async fn preview_live_import_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<LiveImportPreview> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // opencode 走读盘 + Missing 状态；单激活应用走快照解析（无 Missing——
        // 文件缺失即无可导入，Ready 空）。
        if app == App::OpenCode {
            let path = live_opencode::opencode_config_path()?;
            preview_opencode_import(&store, app, &path)
        } else {
            preview_single_activate(&store, app)
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("preview_opencode_import task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::mem;
    use crate::provider::import_live::tests::opencode_live_json;

    // ---------------- 导入预览 ----------------

    /// 预览提取 name / endpoint / 密钥布尔：带 name 用 name，缺 name 用 key；
    /// baseURL 取 options.baseURL；apiKey 存在 → has_secret=true。键序字母序。
    #[test]
    fn preview_lists_entries_with_name_endpoint_and_secret() {
        let entries = preview_opencode_import_text(opencode_live_json(), &HashSet::new());
        assert_eq!(entries.len(), 2, "字母序：deepseek 先于 kimi");
        let ds = &entries[0];
        assert_eq!(ds.key, "deepseek");
        assert_eq!(ds.name, "DeepSeek", "entry.name 作显示名");
        assert_eq!(ds.base_url, "https://api.deepseek.com");
        assert!(ds.has_secret, "options.apiKey 非空 → 含密钥");
        assert!(ds.is_new, "DB 无此 liveKey → 新建");
        let kimi = &entries[1];
        assert_eq!(kimi.key, "kimi");
        assert_eq!(kimi.name, "kimi", "无 name → key 作显示名");
        assert_eq!(kimi.base_url, "https://api.moonshot.cn");
        assert!(!kimi.has_secret, "无 apiKey → 不含密钥");
    }

    /// 空 name（`"name": ""`）导入与预览同判：回退 key（原导入存 ""、预览回退
    /// key，两路漂移——现共用 entry_display_name，#67）。
    #[test]
    fn import_and_preview_agree_on_empty_name_falling_back_to_key() {
        let live = r#"{
          "provider": {
            "blank": {
              "npm": "@ai-sdk/openai-compatible",
              "name": "",
              "options": { "baseURL": "https://x.dev", "apiKey": "sk-x" }
            }
          }
        }"#;
        let entries = preview_opencode_import_text(live, &HashSet::new());
        assert_eq!(entries[0].name, "blank", "预览：空 name → key");
        let s = mem();
        import_live::import_opencode_from_live_text(&s, App::OpenCode, live, &HashMap::new())
            .unwrap();
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        assert_eq!(
            providers[0].name, "blank",
            "导入与预览同一显示名（空 name → key，不再存空串）"
        );
    }

    /// 「新建 vs 更新」判定与导入一致：existing_keys 按 liveKey 集合判定。
    #[test]
    fn preview_classifies_new_vs_update() {
        let existing: HashSet<String> = ["deepseek".to_string()].into_iter().collect();
        let entries = preview_opencode_import_text(opencode_live_json(), &existing);
        assert!(!entries[0].is_new, "已有同 liveKey → 更新");
        assert!(entries[1].is_new, "无此 liveKey → 新建");
    }

    /// 防泄漏回归护栏：预览载荷序列化后绝不包含密钥值（apiKey 只出布尔）。
    #[test]
    fn preview_output_never_contains_secret_value() {
        let entries = preview_opencode_import_text(opencode_live_json(), &HashSet::new());
        let json = serde_json::to_string(&entries).expect("preview entries serialize");
        assert!(!json.contains("sk-x"), "preview 载荷不得携带密钥值: {json}");
    }

    /// headers 也能携带凭据（Authorization 等）→ 计入 has_secret；空值不算。
    #[test]
    fn preview_detects_headers_secret() {
        let live = r#"{
          "provider": {
            "h1": { "options": { "headers": { "Authorization": "Bearer abc" } } },
            "h2": { "options": { "headers": { "X-Empty": "" } } },
            "h3": { "options": { "apiKey": "" } }
          }
        }"#;
        let entries = preview_opencode_import_text(live, &HashSet::new());
        assert_eq!(entries.len(), 3);
        assert!(entries[0].has_secret, "headers 非空值 → 含密钥");
        assert!(!entries[1].has_secret, "headers 空值 → 不含");
        assert!(!entries[2].has_secret, "apiKey 空串 → 不含");
    }

    /// 无 provider 段 / 损坏 JSON5 / 非对象根 → 空 Vec（与导入「静默 0 条」一致，
    /// preview 与 import 语义不得分叉）。
    #[test]
    fn preview_empty_or_unparseable_live_is_empty() {
        for live in [r#"{"model":"x"}"#, "{bad", "[1,2]", ""] {
            let entries = preview_opencode_import_text(live, &HashSet::new());
            assert!(entries.is_empty(), "输入 {live:?} 应 → 空 Vec");
        }
    }

    /// 薄壳：文件不存在 → Missing 变体（带完整路径），不是 Err。
    #[test]
    fn preview_shell_missing_file_is_missing_variant() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        match preview_opencode_import(&s, App::OpenCode, &path).expect("missing is Ok") {
            LiveImportPreview::Missing { path: shown } => {
                assert_eq!(shown, path.display().to_string());
            }
            LiveImportPreview::Ready { .. } => panic!("缺文件应 Missing"),
        }
    }

    /// 薄壳：文件存在 → Ready + 条目（经真实读盘路径）。
    #[test]
    fn preview_shell_ready_with_seeded_file() {
        let s = mem();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("opencode.json");
        std::fs::write(&path, opencode_live_json()).expect("write live file");
        match preview_opencode_import(&s, App::OpenCode, &path).expect("ready is Ok") {
            LiveImportPreview::Ready { entries } => {
                assert_eq!(entries.len(), 2);
                assert!(entries.iter().all(|e| e.is_new), "空 DB → 全部新建");
            }
            LiveImportPreview::Missing { .. } => panic!("文件存在应 Ready"),
        }
    }
}
