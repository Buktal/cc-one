//! live 配置文件反向导入域：附加模式（OpenCode）写盘/移除/导入、单激活应用
//! 快照导入、「从 live 导入」预览。纯转换逻辑在 `provider::import_live` /
//! `provider::live_opencode`，本模块是文件 IO + store 写库的命令层薄壳。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tauri::State;

use super::providers::emit_providers_changed;
use super::AppState;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider, ProviderCategory};
use crate::provider::{import_live, live, live_codex, live_gemini, live_grok, live_opencode};

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
/// `meta.liveManaged = false`（保留 liveKey，便于再加回来）。无 liveKey（从未写
/// 盘）→ 无操作，原样返回。
fn remove_opencode_from_live(store: &Store, provider: Provider) -> AppResult<Provider> {
    let Some(key) = live_opencode::meta_live_key(&provider.meta) else {
        return Ok(provider);
    };
    let path = live_opencode::opencode_config_path()?;
    live_opencode::remove_opencode_provider(&path, &key)?;
    let updated = Provider {
        meta: live_opencode::with_meta_live_state(&provider.meta, &key, false)?,
        ..provider
    };
    store.save_provider(updated)
}

/// 附加模式「从配置文件导入」：把 opencode.json 的 `provider.<key>` 反向导入 DB。
/// 读盘薄壳——核心逻辑在 [`import_opencode_from_live_text`]（可测，不碰文件系统）。
fn import_opencode_from_live(
    store: &Store,
    app: App,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let path = live_opencode::opencode_config_path()?;
    let live_text = live::read_live_settings(&path)?;
    import_opencode_from_live_text(store, app, &live_text, name_overrides)
}

/// 单激活应用「从 live 配置文件导入」（ADR-0012 泛化）：读该应用的 live 文件(s)
/// 反向解析为快照（至多 1 个），upsert 进库（按 name 去重）。opencode 走
/// [`import_opencode_from_live`]。返回写入条数。`name_overrides` = 预览列表里
/// 用户行内改过的名字（key → name；单激活 key == name）。
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
    import_live::upsert_by_name(store, app, &snap, &crate::time::now_iso())
}

/// 按 app 读 live 文件文本（顺序固定：claude=[settings.json]，codex=[config.toml,
/// auth.json]，gemini=[.env, settings.json]，grok=[config.toml]）。opencode 无单份
/// live 配置概念 → `None`。路径解析错误 → `Err`（与既有读盘语义一致）。快照与
/// 片段提取共用这一份「app → 路径」映射（单一事实来源）。
pub(super) fn read_live_texts(app: App) -> AppResult<Option<Vec<String>>> {
    let paths: Vec<AppResult<PathBuf>> = match app {
        App::Claude => vec![live::claude_settings_path()],
        App::Codex => vec![
            live_codex::codex_config_path(),
            live_codex::codex_auth_path(),
        ],
        App::Gemini => vec![
            live_gemini::gemini_env_path(),
            live_gemini::gemini_settings_path(),
        ],
        App::Grok => vec![live_grok::grok_config_path()],
        App::OpenCode => return Ok(None),
    };
    let mut texts = Vec::with_capacity(paths.len());
    for p in paths {
        texts.push(live::read_live_settings(&p?)?);
    }
    Ok(Some(texts))
}

/// 按 app 读 live 文件(s) + 反向解析为快照。opencode → `unreachable`（走
/// opencode 专属路径）。
fn read_live_snapshot(app: App) -> AppResult<Option<import_live::LiveImportSnapshot>> {
    let Some(texts) = read_live_texts(app)? else {
        return Ok(None);
    };
    Ok(match app {
        App::Claude => import_live::claude_live_to_snapshot(&texts[0]),
        App::Codex => import_live::codex_live_to_snapshot(&texts[0], &texts[1]),
        App::Gemini => import_live::gemini_live_to_snapshot(&texts[0], &texts[1]),
        App::Grok => import_live::grok_live_to_snapshot(&texts[0]),
        App::OpenCode => unreachable!("opencode 走 import_opencode_from_live"),
    })
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

/// import 的核心逻辑（可测）：给定 opencode.json 文本，把 `provider.<key>` 反向
/// 导入 DB。每个 key → 一条 Provider：已存在同 liveKey → 更新 settings_config +
/// name（保留 id/展示字段）；否则新建（空 id 交 save_provider 自动生成 hex id +
/// sort_index + updated_at）。反复 import 按 liveKey 去重，不产生重复。返回导入/
/// 更新条数。
fn import_opencode_from_live_text(
    store: &Store,
    app: App,
    live_text: &str,
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let entries = live_opencode::provider_entries(live_text);
    if entries.is_empty() {
        return Ok(0);
    }
    // 现有 provider 按 liveKey 索引——按配置文件原 key 判「已存在」。
    let mut by_live_key: HashMap<String, Provider> = HashMap::new();
    for p in store.list_providers_for(app)? {
        if let Some(k) = live_opencode::meta_live_key(&p.meta) {
            by_live_key.insert(k, p);
        }
    }
    let mut count = 0u32;
    for (key, entry) in entries {
        let settings_config = serde_json::to_string(&entry)?;
        // 预览列表的行内改名优先（key → name 覆盖），否则 entry.name 非空
        // 优先、缺失或空串 → key（与预览同一推导，单一事实来源）。
        let display_name = name_overrides
            .get(&key)
            .cloned()
            .unwrap_or_else(|| live_opencode::entry_display_name(&entry, &key));
        let provider = match by_live_key.get(&key) {
            Some(existing) => Provider {
                name: display_name,
                settings_config,
                meta: live_opencode::with_meta_live_state(&existing.meta, &key, true)?,
                ..existing.clone()
            },
            None => Provider {
                id: String::new(),
                name: display_name,
                website_url: String::new(),
                category: ProviderCategory::Custom,
                app,
                icon: String::new(),
                icon_color: String::new(),
                sort_index: 0,
                notes: String::new(),
                settings_config,
                meta: live_opencode::with_meta_live_state("", &key, true)?,
                updated_at: String::new(),
            },
        };
        store.save_provider(provider)?;
        count += 1;
    }
    Ok(count)
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

    /// 一份带两个 provider（一个带 name、一个不带）+ 顶层用户字段的 opencode.json。
    fn opencode_live_json() -> &'static str {
        r#"{
          "model": "deepseek/deepseek-chat",
          "provider": {
            "deepseek": {
              "npm": "@ai-sdk/openai-compatible",
              "name": "DeepSeek",
              "options": { "baseURL": "https://api.deepseek.com", "apiKey": "sk-x" }
            },
            "kimi": {
              "npm": "@ai-sdk/openai-compatible",
              "options": { "baseURL": "https://api.moonshot.cn" }
            }
          }
        }"#
    }

    /// 导入把 provider.<key> 反向落库：新建（空 id → 自动 hex）、liveKey=原 key、
    /// liveManaged=true；display name 取 entry.name，无 name 则取 key。
    #[test]
    fn import_creates_providers_with_live_key_and_managed_flag() {
        let s = mem();
        let n = import_opencode_from_live_text(
            &s,
            App::OpenCode,
            opencode_live_json(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 2);
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        assert_eq!(providers.len(), 2);
        let by_name: HashMap<String, Provider> = providers
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();
        // 带 name 的 → 用 name。
        let ds = by_name.get("DeepSeek").expect("entry.name 作 display name");
        assert_eq!(
            live_opencode::meta_live_key(&ds.meta).as_deref(),
            Some("deepseek"),
            "liveKey = 配置文件原 key"
        );
        assert_eq!(live_opencode::meta_live_managed(&ds.meta), Some(true));
        // 不带 name 的 → 用 key。
        let kimi = by_name.get("kimi").expect("无 name → key 作 display name");
        assert_eq!(
            live_opencode::meta_live_key(&kimi.meta).as_deref(),
            Some("kimi")
        );
        // settingsConfig 是 entry 子树（npm/options）。
        let sc: serde_json::Value = serde_json::from_str(&ds.settings_config).unwrap();
        assert_eq!(sc["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(sc["options"]["baseURL"], "https://api.deepseek.com");
    }

    /// 反复 import 同一文件 → 按 liveKey 匹配更新，不产生重复。
    #[test]
    fn import_updates_existing_same_live_key_no_duplicate() {
        let s = mem();
        import_opencode_from_live_text(&s, App::OpenCode, opencode_live_json(), &HashMap::new())
            .unwrap();
        let n = import_opencode_from_live_text(
            &s,
            App::OpenCode,
            opencode_live_json(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 2, "第二次仍处理 2 条（按 liveKey 更新）");
        assert_eq!(
            s.list_providers_for(App::OpenCode).unwrap().len(),
            2,
            "不产生重复"
        );
    }

    /// 行内改名覆盖：nameOverrides（key → name）优先于 entry.name / key。
    #[test]
    fn import_respects_name_overrides() {
        let s = mem();
        let overrides = HashMap::from([("deepseek".to_string(), "DS 直连".to_string())]);
        let n = import_opencode_from_live_text(&s, App::OpenCode, opencode_live_json(), &overrides)
            .unwrap();
        assert_eq!(n, 2);
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        let ds = providers
            .iter()
            .find(|p| live_opencode::meta_live_key(&p.meta).as_deref() == Some("deepseek"))
            .expect("deepseek 存在");
        assert_eq!(ds.name, "DS 直连", "覆盖名优先于 entry.name");
        // 未被覆盖的 key 仍走 entry.name / key 规则。
        let kimi = providers
            .iter()
            .find(|p| live_opencode::meta_live_key(&p.meta).as_deref() == Some("kimi"))
            .expect("kimi 存在");
        assert_eq!(kimi.name, "kimi");
    }

    /// 无 provider 段 → 0 条（顶层用户字段 model 等被忽略，不报错）。
    #[test]
    fn import_empty_providers_section_is_zero() {
        let s = mem();
        let n =
            import_opencode_from_live_text(&s, App::OpenCode, r#"{"model":"x"}"#, &HashMap::new())
                .unwrap();
        assert_eq!(n, 0);
        assert!(s.list_providers_for(App::OpenCode).unwrap().is_empty());
    }

    // ---------------- 单激活应用「从 live 导入」（ADR-0012 泛化）----------------

    /// claude live → 快照 → upsert：按 name（base_url host）新建 Provider。
    #[test]
    fn single_activate_import_claude_live_creates_provider() {
        let s = mem();
        let snap = import_live::claude_live_to_snapshot(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"kimi"}}"#,
        )
        .unwrap();
        let n =
            import_live::upsert_by_name(&s, App::Claude, &snap, "2026-08-13T00:00:00Z").unwrap();
        assert_eq!(n, 1);
        let providers = s.list_providers_for(App::Claude).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(
            providers[0].name, "moonshot",
            "注册域去 TLD（host_of 规则）"
        );
        let sc: serde_json::Value = serde_json::from_str(&providers[0].settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "kimi");
    }

    /// 反复导入同 name → 更新不重复（单激活的 liveKey 替代：按 name 去重）。
    #[test]
    fn single_activate_import_dedupes_by_name() {
        let s = mem();
        let snap =
            import_live::claude_live_to_snapshot(r#"{"env":{"ANTHROPIC_MODEL":"m1"}}"#).unwrap();
        import_live::upsert_by_name(&s, App::Claude, &snap, "t1").unwrap();
        import_live::upsert_by_name(&s, App::Claude, &snap, "t2").unwrap();
        assert_eq!(
            s.list_providers_for(App::Claude).unwrap().len(),
            1,
            "同 name 不产生重复"
        );
    }

    /// 无可导入内容（无受控键）→ 0 条。
    #[test]
    fn single_activate_import_empty_live_is_zero() {
        let s = mem();
        let snap = import_live::claude_live_to_snapshot(r#"{"permissions":{"allow":["Bash"]}}"#);
        assert!(snap.is_none(), "无受控内容 → 无可导入");
        assert!(s.list_providers_for(App::Claude).unwrap().is_empty());
    }

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
        import_opencode_from_live_text(&s, App::OpenCode, live, &HashMap::new()).unwrap();
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
