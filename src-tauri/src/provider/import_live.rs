//! 单激活应用「从 live 配置文件导入」：live → Provider 快照 反向解析。
//!
//! opencode 的「从配置文件导入」泛化到 claude / codex / gemini / grok
//! （ADR-0012）。它们 live 是**单份配置**（settings.json / config.toml /
//! .env + settings.json / config.toml），**一份 live → 至多 1 个 Provider**
//! （0 个 = 文件缺失 / 空 / 无可识别身份内容），与 opencode 的
//! `provider.<key>` map（多条共存）本质不同——opencode 仍走
//! `commands::import_opencode_from_live_text`，本模块只管 4 个单激活应用。
//!
//! 拆分规则 = 各应用写盘的反向镜像（受控字段表是单一事实来源，见 ADR-0005 /
//! ADR-0010）：
//! - **受控字段 → Provider.settings_config**（写盘时供应商接管的部分）。
//! - **可共享键 → snippet_candidates**（跨供应商共享候选，导入后提示「提取为
//!   通用片段」，T6）。按各应用片段语义：claude / gemini 的片段只在受控字段内
//!   补缺失（`snippet::merge_snippet_into_settings` 只认 `CONTROLLED_FIELDS`），
//!   故候选 = 受控字段里非敏感的部分；codex / grok 的片段在写盘层补缺失
//!   （`merge_codex_snippet` / `merge_grok_snippet` 用 `fill_missing_table`），
//!   故候选 = 非受控顶层键。
//!
//! 去重：单激活应用 meta 无 opencode 的 liveKey（`live_opencode::meta_live_key`
//! 是 opencode 专属），按 **name** 去重——同 app 同 name → 更新
//! settings_config（保留 id / 展示字段），否则新建。name = live 里 base_url 的
//! 注册域（去常见 TLD，用户认得出是哪个供应商），无 base_url → 应用名
//! （"Claude" 等，数据非 i18n）。
//!
//! 所有转换函数是纯函数（测试接缝）：live 文本 → 快照，不碰文件系统 / DB；
//! 文件 IO 与 store 写库在命令层薄壳（`commands.rs`）。

use std::collections::HashMap;

use serde_json::Value;
use toml_edit::DocumentMut;

use crate::db::Store;
use crate::error::AppResult;
use crate::model::{App, Provider, ProviderCategory};
use crate::provider::live::{self, CONTROLLED_FIELDS};
use crate::provider::live_codex::CODEX_CONTROLLED_FIELDS;
use crate::provider::snippet::is_sensitive_config_key;

/// 单激活应用从 live 反向解析出的一个导入快照（至多一个）。
pub struct LiveImportSnapshot {
    /// Provider.name（去重键）：base_url 的注册域（[`name_from_base_url`]），
    /// 无 base_url → 应用名。
    pub name: String,
    /// settingsConfig（受控字段子集，写盘方向的反向镜像）。
    pub settings_config: String,
    /// 完整 base_url（预览展示用；无 → 空串）。
    pub base_url: String,
    /// settingsConfig 是否携带凭据（预览「含密钥」徽标；密钥值不跨边界）。
    pub has_secret: bool,
    /// 可共享键候选（导入后提示「提取为通用片段」，T6）。
    pub snippet_candidates: Vec<String>,
}

/// 常见顶级域（gTLD + 常用 ccTLD），命名规则「去 TLD 取注册域」用：host 的
/// 最后一段命中此表 → 名字取倒数第二段（`opencode.ai` → `opencode`、
/// `api.anthropic.com` → `anthropic`）。**只按末段判定**：复合后缀（`co.uk`
/// 类）不在表内，`x.co.uk` 会错取 `co`、大小写敏感、带端口不命中——均低频
/// 且名字在导入对话框里可改，表够用即可，不追求 public_suffix 全量。
const COMMON_TLDS: &[&str] = &[
    "com", "net", "org", "io", "ai", "dev", "app", "co", "uk", "us", "jp", "cn", "de", "fr", "ru",
    "in", "xyz", "me", "gg", "sh", "to", "tv", "cc", "biz", "info", "tech",
];

/// base_url → 显示名（name 推导用，单激活应用导入）。末段命中
/// [`COMMON_TLDS`] → 注册域（`https://api.moonshot.cn/anthropic` →
/// `moonshot`）；否则 host 原样（localhost / IP / 内网域名不猜）；无协议 /
/// 空 → `None`。
fn name_from_base_url(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1)?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|s| !s.is_empty())?;
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() >= 2 && COMMON_TLDS.contains(&labels[labels.len() - 1]) {
        Some(labels[labels.len() - 2].to_string())
    } else {
        Some(host.to_string())
    }
}

/// `.env` 文本 → 键值对（纯函数，gemini 反向解析；对齐
/// `live_gemini::serialize_env_file` 的 `KEY=VALUE` 格式）。忽略空行与 `#` 注释
/// 行；`=` 前有键即取值（值原样保留，含 `=` 后面的内容）。坏行（无 `=` / 空键）
/// 跳过——容错：单行坏数据不让整个 .env 导入失败。
pub fn parse_env_file(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        out.insert(k.to_string(), v.to_string());
    }
    out
}

/// claude：`settings.json` → 快照。受控字段（`CONTROLLED_FIELDS`）→
/// settings_config，剥内部 meta 键；无可识别受控内容 → `None`。
pub fn claude_live_to_snapshot(live: &str) -> Option<LiveImportSnapshot> {
    let obj = live::parse_live_or_empty(live).ok()?;
    let map = obj.as_object()?;
    let mut settings: serde_json::Map<String, Value> = serde_json::Map::new();
    for key in CONTROLLED_FIELDS {
        if let Some(v) = map.get(*key) {
            settings.insert((*key).to_string(), v.clone());
        }
    }
    live::strip_internal_keys(&mut settings);
    if settings.is_empty() {
        return None;
    }
    let env = settings.get("env").and_then(|e| e.as_object());
    let base_url = env
        .and_then(|e| e.get("ANTHROPIC_BASE_URL"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let has_secret = env.is_some_and(|e| e.keys().any(|k| is_sensitive_config_key(k)));
    // 片段候选：受控字段里非敏感的部分（env 非敏感键 + 顶层开关）。
    let mut snippet_candidates = Vec::new();
    if let Some(env_map) = env {
        for k in env_map.keys() {
            if !is_sensitive_config_key(k) {
                snippet_candidates.push(k.clone());
            }
        }
    }
    for key in CONTROLLED_FIELDS {
        if *key != "env" && settings.contains_key(*key) {
            snippet_candidates.push((*key).to_string());
        }
    }
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Claude".to_string()),
        settings_config: serde_json::to_string_pretty(&settings).ok()?,
        base_url,
        has_secret,
        snippet_candidates,
    })
}

/// codex：`config.toml` + `auth.json` → 快照。settings_config =
/// `{"auth":{"OPENAI_API_KEY":...}, "config":"<只含 CODEX_CONTROLLED_FIELDS 键的
/// TOML>"}`；auth 只取 `OPENAI_API_KEY`（trim 非空才有，其余登录态字段不导）。
/// 无可识别受控内容 → `None`。
pub fn codex_live_to_snapshot(config_toml: &str, auth_json: &str) -> Option<LiveImportSnapshot> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    // 受控键子集 TOML（写盘 `merge_codex_config` 的反向）。
    let mut out = DocumentMut::new();
    for key in CODEX_CONTROLLED_FIELDS {
        if let Some(item) = doc.get(key) {
            out.insert(key, item.clone());
        }
    }
    // auth：只取 OPENAI_API_KEY（非空）。
    let mut auth: serde_json::Map<String, Value> = serde_json::Map::new();
    if !auth_json.trim().is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(auth_json) {
            if let Some(obj) = v.as_object() {
                if let Some(k) = obj
                    .get("OPENAI_API_KEY")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    auth.insert("OPENAI_API_KEY".into(), Value::String(k.to_string()));
                }
            }
        }
    }
    if out.as_table().is_empty() && auth.is_empty() {
        return None;
    }
    // base_url：`model_providers.custom.base_url`。
    let base_url = doc
        .get("model_providers")
        .and_then(|t| t.get("custom"))
        .and_then(|t| t.get("base_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let has_secret = !auth.is_empty();
    // 片段候选：config.toml 非受控顶层键（mcp_servers / web_search 等）。
    let snippet_candidates = doc
        .as_table()
        .iter()
        .filter(|(k, _)| !CODEX_CONTROLLED_FIELDS.contains(k))
        .map(|(k, _)| k.to_string())
        .collect();
    let settings = serde_json::json!({ "auth": auth, "config": out.to_string() });
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Codex".to_string()),
        settings_config: serde_json::to_string_pretty(&settings).ok()?,
        base_url,
        has_secret,
        snippet_candidates,
    })
}

/// gemini：`.env` + `settings.json` → 快照。settings_config = `{"env":{...}}` +
/// `config`（settings.json 顶层非内部键；`security` 认证标记归写盘推导，不导）。
/// env 整块是受控单位。无可识别内容 → `None`。
pub fn gemini_live_to_snapshot(env_text: &str, settings_json: &str) -> Option<LiveImportSnapshot> {
    let env = parse_env_file(env_text);
    let config = if settings_json.trim().is_empty() {
        None
    } else {
        let v = serde_json::from_str::<Value>(settings_json).ok()?;
        let obj = v.as_object()?;
        let mut map = obj.clone();
        live::strip_internal_keys(&mut map);
        map.remove("security");
        if map.is_empty() {
            None
        } else {
            Some(map)
        }
    };
    if env.is_empty() && config.is_none() {
        return None;
    }
    let base_url = env
        .get("GOOGLE_GEMINI_BASE_URL")
        .map(String::as_str)
        .unwrap_or("")
        .to_string();
    let has_secret = env.keys().any(|k| is_sensitive_config_key(k));
    // 片段候选：只列 env 能提取的键（非敏感非端点）——settings.json 的非受控键
    // 进片段零效果（`gemini_extract_snippet` 只提取 env），列入只会让「提取」提示
    // 空欢喜。与 claude 同规则：受控字段里非敏感的部分。
    let snippet_candidates = env
        .keys()
        .filter(|k| !is_sensitive_config_key(k) && k.as_str() != "GOOGLE_GEMINI_BASE_URL")
        .cloned()
        .collect();
    let mut settings = serde_json::Map::new();
    settings.insert(
        "env".to_string(),
        Value::Object(
            env.into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        ),
    );
    if let Some(c) = config {
        settings.insert("config".to_string(), Value::Object(c));
    }
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Gemini".to_string()),
        settings_config: serde_json::to_string_pretty(&settings).ok()?,
        base_url,
        has_secret,
        snippet_candidates,
    })
}

/// grok：`config.toml` → 快照。settings_config = `{"config":"<仅 [model."cc-one"]
/// profile 块的 TOML>"}`（写盘 `merge_grok_config` 的反向；models.default 指针由
/// 写盘层补）。无 cc-one profile（登录态版）→ `None`。
pub fn grok_live_to_snapshot(config_toml: &str) -> Option<LiveImportSnapshot> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    let profile = doc.get("model").and_then(|t| t.get("cc-one")).cloned()?;
    // 只保留 [model."cc-one"] 块（用户其它 profile / mcp_servers 不进 settings_config）。
    // model 表标 implicit——只渲染 [model."cc-one"]、不产出孤立的 [model] 头（与
    // 写盘 `merge_grok_config` 同一构造）。
    let mut out = DocumentMut::new();
    let mut model = toml_edit::Table::new();
    model.insert("cc-one", profile);
    model.set_implicit(true);
    out.as_table_mut()
        .insert("model", toml_edit::Item::Table(model));
    let base_url = doc
        .get("model")
        .and_then(|t| t.get("cc-one"))
        .and_then(|t| t.get("base_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let has_secret = doc
        .get("model")
        .and_then(|t| t.get("cc-one"))
        .and_then(|t| t.get("api_key"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    // 片段候选：顶层非 model/models 键（mcp_servers 等非受控共享键）。
    let snippet_candidates = doc
        .as_table()
        .iter()
        .filter(|(k, _)| *k != "model" && *k != "models")
        .map(|(k, _)| k.to_string())
        .collect();
    let settings = serde_json::json!({ "config": out.to_string() });
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Grok".to_string()),
        settings_config: serde_json::to_string_pretty(&settings).ok()?,
        base_url,
        has_secret,
        snippet_candidates,
    })
}

/// 从 live 提取「可共享键」为片段内容（T6，ADR-0012 consequence L30——导入后
/// 检测非身份共享键、非静默提示「提取为通用片段」）。按各应用片段语义：
/// - claude：片段只在受控字段内合并（`snippet::merge_snippet_into_settings` 只
///   认 `CONTROLLED_FIELDS`）→ 提取 env 非敏感键 + 顶层开关。
/// - gemini：同上，但只提取 env（settings.json 的非受控键进片段零效果）——
///   排除凭据键与端点键。
/// - codex / grok：片段在写盘层补缺失（`fill_missing_table`）→ 提取非受控顶层
///   键（codex = 非 `CODEX_CONTROLLED_FIELDS`；grok = 非 model / models）。
///
/// 无可提取 → `None`。
pub fn claude_extract_snippet(live: &str) -> Option<String> {
    let obj = live::parse_live_or_empty(live).ok()?;
    let map = obj.as_object()?;
    let mut out: serde_json::Map<String, Value> = serde_json::Map::new();
    if let Some(env) = map.get("env").and_then(|e| e.as_object()) {
        let mut env_out = serde_json::Map::new();
        for (k, v) in env {
            if !is_sensitive_config_key(k) {
                env_out.insert(k.clone(), v.clone());
            }
        }
        if !env_out.is_empty() {
            out.insert("env".into(), Value::Object(env_out));
        }
    }
    for key in CONTROLLED_FIELDS {
        if *key != "env" {
            if let Some(v) = map.get(*key) {
                out.insert((*key).to_string(), v.clone());
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(serde_json::to_string_pretty(&out).ok()?)
    }
}

pub fn gemini_extract_snippet(env_text: &str) -> Option<String> {
    let env = parse_env_file(env_text);
    let mut env_out = serde_json::Map::new();
    for (k, v) in env {
        // 空值跳过：无可共享内容，且 set 校验拒空值（gemini env 值须非空）——
        // 提取与手动保存同判，live 的 `KEY=` 空值行不进片段。
        if !is_sensitive_config_key(&k) && k != "GOOGLE_GEMINI_BASE_URL" && !v.trim().is_empty() {
            env_out.insert(k, Value::String(v));
        }
    }
    if env_out.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "env": env_out }).to_string())
    }
}

pub fn codex_extract_snippet(config_toml: &str) -> Option<String> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    let mut out = DocumentMut::new();
    for (k, item) in doc.as_table().iter() {
        if !CODEX_CONTROLLED_FIELDS.contains(&k) {
            out.insert(k, item.clone());
        }
    }
    if out.as_table().is_empty() {
        None
    } else {
        Some(out.to_string())
    }
}

pub fn grok_extract_snippet(config_toml: &str) -> Option<String> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    let mut out = DocumentMut::new();
    for (k, item) in doc.as_table().iter() {
        if k != "model" && k != "models" {
            out.insert(k, item.clone());
        }
    }
    if out.as_table().is_empty() {
        None
    } else {
        Some(out.to_string())
    }
}

/// 把提取的片段内容合并进现有片段（只补缺失，沿用 ADR-0010 语义——已有键不
/// 覆盖）：claude / gemini（JSON）用 `snippet::merge_snippet_into_settings`（现有
/// 片段为 target、提取为补丁）；codex / grok（TOML）用 `fill_missing_table`。
/// 返回合并后的片段文本。
pub fn merge_extracted_snippet(app: App, existing: &str, extracted: &str) -> AppResult<String> {
    match app {
        App::Claude | App::Gemini => {
            crate::provider::snippet::merge_snippet_into_settings(existing, extracted)
        }
        App::Codex | App::Grok => {
            let mut doc = live::parse_toml_or_empty(existing, "existing snippet")?;
            let ext = live::parse_toml_or_empty(extracted, "extracted snippet")?;
            live::fill_missing_table(doc.as_table_mut(), ext.as_table());
            Ok(doc.to_string())
        }
        App::OpenCode => Ok(extracted.to_string()),
    }
}

/// 把快照 upsert 进库（按 name 去重）：同 app 同 name → 更新 settings_config /
/// name / updated_at（保留 id / 展示字段），否则新建（空 id 交 `save_provider`
/// 生成 hex）。返回写入条数（0 或 1）。
pub fn upsert_by_name(
    store: &Store,
    app: App,
    snap: &LiveImportSnapshot,
    now_iso: &str,
) -> AppResult<u32> {
    let existing = store.list_providers_for(app)?;
    if let Some(prev) = existing.iter().find(|p| p.name == snap.name) {
        let provider = Provider {
            name: snap.name.clone(),
            settings_config: snap.settings_config.clone(),
            updated_at: now_iso.to_string(),
            ..prev.clone()
        };
        store.save_provider(provider)?;
        return Ok(1);
    }
    let provider = Provider {
        id: String::new(),
        name: snap.name.clone(),
        website_url: String::new(),
        category: ProviderCategory::Custom,
        app,
        icon: String::new(),
        icon_color: String::new(),
        sort_index: 0,
        notes: String::new(),
        settings_config: snap.settings_config.clone(),
        meta: "{}".into(),
        updated_at: now_iso.to_string(),
    };
    store.save_provider(provider)?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV: &str = "GOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com/v1beta\nGEMINI_API_KEY=sk-test\nGEMINI_MODEL=gemini-2.5-flash\n";

    #[test]
    fn parse_env_file_reads_key_value_lines() {
        let env = parse_env_file(ENV);
        assert_eq!(
            env.get("GOOGLE_GEMINI_BASE_URL").map(String::as_str),
            Some("https://generativelanguage.googleapis.com/v1beta")
        );
        assert_eq!(
            env.get("GEMINI_API_KEY").map(String::as_str),
            Some("sk-test")
        );
        assert_eq!(
            env.get("GEMINI_MODEL").map(String::as_str),
            Some("gemini-2.5-flash")
        );
    }

    #[test]
    fn parse_env_file_skips_comments_and_blank_lines() {
        let env = parse_env_file("# 注释\n\nKEY=value\n");
        assert_eq!(env.len(), 1);
        assert_eq!(env.get("KEY").map(String::as_str), Some("value"));
    }

    #[test]
    fn parse_env_file_tolerates_bad_lines() {
        let env = parse_env_file("NO_EQUALS\n=novalue\nKEY=1=2\n");
        assert_eq!(
            env.get("KEY").map(String::as_str),
            Some("1=2"),
            "值含 = 原样保留"
        );
        assert!(!env.contains_key("NO_EQUALS"));
        assert!(!env.contains_key(""));
    }

    #[test]
    fn claude_extracts_controlled_fields_only() {
        let live = r#"{
            "env": {"ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic", "ANTHROPIC_AUTH_TOKEN": "sk-x", "ANTHROPIC_MODEL": "kimi-k2"},
            "includeCoAuthoredBy": false,
            "permissions": {"allow": ["Bash"]},
            "hooks": {"PreToolUse": [{"matcher": "*"}]},
            "mcpServers": {"filesystem": {"command": "npx"}}
        }"#;
        let snap = claude_live_to_snapshot(live).expect("有受控内容");
        assert_eq!(snap.name, "moonshot");
        assert!(snap.has_secret, "env 含 ANTHROPIC_AUTH_TOKEN（凭据）");
        let sc: Value = serde_json::from_str(&snap.settings_config).unwrap();
        let sc_obj = sc.as_object().unwrap();
        assert!(sc_obj.contains_key("env"));
        assert!(sc_obj.contains_key("includeCoAuthoredBy"));
        assert!(
            !sc_obj.contains_key("permissions"),
            "非受控不进 settings_config"
        );
        assert!(!sc_obj.contains_key("hooks"));
        assert!(!sc_obj.contains_key("mcpServers"));
        assert!(snap
            .snippet_candidates
            .contains(&"ANTHROPIC_MODEL".to_string()));
        assert!(snap
            .snippet_candidates
            .contains(&"includeCoAuthoredBy".to_string()));
        assert!(
            !snap
                .snippet_candidates
                .iter()
                .any(|k| k == "ANTHROPIC_AUTH_TOKEN"),
            "凭据不进候选"
        );
    }

    #[test]
    fn claude_empty_or_identityless_is_none() {
        assert!(claude_live_to_snapshot("").is_none());
        assert!(
            claude_live_to_snapshot(r#"{"permissions": {"allow": ["Bash"]}}"#).is_none(),
            "无受控内容"
        );
    }

    #[test]
    fn claude_fallback_name_without_base_url() {
        let snap = claude_live_to_snapshot(r#"{"env": {"ANTHROPIC_MODEL": "m"}}"#).expect("有 env");
        assert_eq!(snap.name, "Claude");
    }

    #[test]
    fn codex_extracts_controlled_toml_and_auth_key() {
        let config = r#"model = "gpt-5"
model_provider = "custom"

[model_providers.custom]
name = "custom"
base_url = "https://api.openai.com/v1"
wire_api = "responses"

[mcp_servers.github]
command = "npx"
"#;
        let auth = r#"{"OPENAI_API_KEY": "sk-codex", "tokens": {"access": "x"}}"#;
        let snap = codex_live_to_snapshot(config, auth).expect("有受控内容");
        assert_eq!(snap.name, "openai");
        assert!(snap.has_secret);
        let sc: Value = serde_json::from_str(&snap.settings_config).unwrap();
        assert_eq!(sc["auth"]["OPENAI_API_KEY"], "sk-codex");
        assert!(sc["auth"].get("tokens").is_none(), "auth 登录态字段不导");
        let out_config = sc["config"].as_str().unwrap();
        assert!(out_config.contains("model_provider"), "受控键进 config");
        assert!(!out_config.contains("mcp_servers"), "非受控不进 config");
        assert_eq!(snap.snippet_candidates, vec!["mcp_servers".to_string()]);
    }

    #[test]
    fn codex_login_only_is_none() {
        // config.toml 无受控键 + auth 无 key（登录态）→ 无可导入。
        let config = r#"[mcp_servers.fs]
command = "npx""#;
        assert!(codex_live_to_snapshot(config, "{}").is_none());
    }

    #[test]
    fn codex_auth_key_only_still_imports() {
        // 空 config + auth 有 key → 有受控内容（auth）。
        let snap =
            codex_live_to_snapshot("", r#"{"OPENAI_API_KEY": "sk-x"}"#).expect("auth 有 key");
        let sc: Value = serde_json::from_str(&snap.settings_config).unwrap();
        assert_eq!(sc["auth"]["OPENAI_API_KEY"], "sk-x");
    }

    #[test]
    fn gemini_extracts_env_and_config() {
        let settings = r#"{"mcpServers": {"fs": {"command": "npx"}}, "security": {"auth": {"selectedType": "gemini-api-key"}}}"#;
        let snap = gemini_live_to_snapshot(ENV, settings).expect("有 env");
        assert_eq!(snap.name, "googleapis");
        assert!(snap.has_secret, "env 含 GEMINI_API_KEY");
        let sc: Value = serde_json::from_str(&snap.settings_config).unwrap();
        assert_eq!(sc["env"]["GEMINI_MODEL"], "gemini-2.5-flash");
        assert!(
            sc["config"].get("mcpServers").is_some(),
            "settings.json 非 security 键进 config"
        );
        assert!(
            sc["config"].get("security").is_none(),
            "security 认证标记不导"
        );
        // 片段候选：只列 env 能提取的键（settings.json 的键进片段零效果，不列）。
        assert_eq!(
            snap.snippet_candidates,
            vec!["GEMINI_MODEL".to_string()],
            "候选 = env 非敏感非端点键"
        );
    }

    #[test]
    fn gemini_has_secret_uses_pattern_not_fixed_key() {
        // has_secret 按凭据模式判定（与 claude 同规则）：任何凭据键都算，不只
        // GEMINI_API_KEY——否则 .env 里其它凭据键不会亮「含密钥」徽标。
        let snap =
            gemini_live_to_snapshot("GEMINI_MODEL=gemini-2.5-flash\nCUSTOM_API_TOKEN=sk-x\n", "")
                .expect("有 env");
        assert!(snap.has_secret, "CUSTOM_API_TOKEN 命中凭据模式");
        let no_secret =
            gemini_live_to_snapshot("GEMINI_MODEL=gemini-2.5-flash\n", "").expect("有 env");
        assert!(!no_secret.has_secret, "无凭据键 → 不含密钥");
    }

    #[test]
    fn gemini_empty_is_none() {
        assert!(gemini_live_to_snapshot("", "").is_none());
    }

    #[test]
    fn grok_extracts_cc_one_profile_only() {
        let config = r#"[models]
default = "cc-one"

[model.cc-one]
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
api_key = "sk-grok"

[model.user]
model = "other"

[mcp_servers.fs]
command = "npx"
"#;
        let snap = grok_live_to_snapshot(config).expect("有 cc-one profile");
        assert_eq!(snap.name, "x");
        assert!(snap.has_secret);
        let sc: Value = serde_json::from_str(&snap.settings_config).unwrap();
        let out_config = sc["config"].as_str().unwrap();
        assert!(out_config.contains("[model.cc-one]"), "cc-one 块进 config");
        assert!(
            !out_config.contains("models.default"),
            "default 指针写盘层补，不导"
        );
        assert!(
            !out_config.contains("[model.user]"),
            "用户 profile 不进 config"
        );
        assert!(!out_config.contains("mcp_servers"));
        assert_eq!(snap.snippet_candidates, vec!["mcp_servers".to_string()]);
    }

    #[test]
    fn grok_login_state_is_none() {
        // 无 cc-one profile（登录态版）→ 无可导入。
        assert!(grok_live_to_snapshot(
            r#"[models]
default = "xai""#
        )
        .is_none());
    }

    #[test]
    fn name_from_base_url_derives_registry_domain() {
        // 注册域规则：去常见 TLD 取倒数第二段。
        assert_eq!(
            name_from_base_url("https://api.moonshot.cn/anthropic").as_deref(),
            Some("moonshot")
        );
        assert_eq!(
            name_from_base_url("https://opencode.ai/zen/go").as_deref(),
            Some("opencode")
        );
        assert_eq!(
            name_from_base_url("https://api.anthropic.com").as_deref(),
            Some("anthropic")
        );
        // 未命中 TLD 表 → host 原样（localhost / IP / 内网域名不猜）。
        assert_eq!(
            name_from_base_url("https://generativelanguage.googleapis.com/v1beta").as_deref(),
            Some("googleapis")
        );
        assert_eq!(
            name_from_base_url("https://nas.local/x").as_deref(),
            Some("nas.local")
        );
        assert_eq!(
            name_from_base_url("https://localhost:3000/x").as_deref(),
            Some("localhost:3000")
        );
        assert_eq!(
            name_from_base_url("https://127.0.0.1/x").as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(name_from_base_url("").as_deref(), None);
        assert_eq!(name_from_base_url("no-protocol").as_deref(), None);
    }

    // ---- T6 提取「可共享键」为片段 ----

    #[test]
    fn claude_extract_gets_non_sensitive_env_and_switches() {
        let live = r#"{
            "env": {"ANTHROPIC_MODEL": "kimi", "ANTHROPIC_AUTH_TOKEN": "sk-x", "ANTHROPIC_BASE_URL": "https://x"},
            "includeCoAuthoredBy": false,
            "permissions": {"allow": ["Bash"]}
        }"#;
        let content = claude_extract_snippet(live).expect("有可提取");
        let v: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "kimi", "非敏感 env 键提取");
        assert!(v["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(), "凭据不提取");
        assert_eq!(
            v["includeCoAuthoredBy"],
            serde_json::json!(false),
            "顶层开关提取"
        );
        assert!(
            v.get("permissions").is_none(),
            "非受控不提取（片段合并只认受控字段）"
        );
    }

    #[test]
    fn claude_extract_nothing_identityless_is_none() {
        assert!(claude_extract_snippet(r#"{"permissions":{"allow":["Bash"]}}"#).is_none());
        assert!(
            claude_extract_snippet(r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk"}}"#).is_none(),
            "全是凭据 → 无"
        );
    }

    #[test]
    fn gemini_extract_excludes_credentials_and_endpoint() {
        let content = gemini_extract_snippet(ENV).expect("有可提取");
        let v: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["env"]["GEMINI_MODEL"], "gemini-2.5-flash");
        assert!(v["env"].get("GEMINI_API_KEY").is_none());
        assert!(
            v["env"].get("GOOGLE_GEMINI_BASE_URL").is_none(),
            "端点键不提取"
        );
    }

    /// live `.env` 的 `KEY=` 空值行不进片段：与手动保存同判（set 校验拒空值），
    /// 提取不得绕过（#60）。
    #[test]
    fn gemini_extract_skips_empty_values() {
        let content = gemini_extract_snippet("GEMINI_MODEL=gemini-2.5-flash\nGEMINI_DEBUG=\n")
            .expect("有可提取");
        let v: Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["env"]["GEMINI_MODEL"], "gemini-2.5-flash");
        assert!(
            v["env"].get("GEMINI_DEBUG").is_none(),
            "空值行不进片段（手动保存同判拒绝）"
        );
        // 只有空值 → 无可提取。
        assert!(gemini_extract_snippet("GEMINI_DEBUG=\n").is_none());
    }

    #[test]
    fn codex_extract_gets_uncontrolled_toml_tables() {
        let config = r#"model = "gpt-5"
[mcp_servers.github]
command = "npx"
[web_search]
provider = "google""#;
        let content = codex_extract_snippet(config).expect("有可提取");
        assert!(content.contains("[mcp_servers.github]"), "非受控表提取");
        assert!(content.contains("[web_search]"));
        assert!(!content.contains("model ="), "身份键不提取");
    }

    #[test]
    fn grok_extract_gets_uncontrolled_tables_only() {
        let config = r#"[models]
default = "cc-one"

[model.cc-one]
model = "grok-4.5"

[mcp_servers.fs]
command = "npx"
"#;
        let content = grok_extract_snippet(config).expect("有可提取");
        assert!(content.contains("[mcp_servers.fs]"));
        assert!(!content.contains("[model.cc-one]"), "身份 profile 不提取");
        assert!(!content.contains("default ="), "models 指针不提取");
    }

    #[test]
    fn merge_extracted_fills_missing_only() {
        // claude：现有片段已有键保留，提取只补缺失。
        let merged = merge_extracted_snippet(
            App::Claude,
            r#"{"env":{"ANTHROPIC_MODEL":"mine"}}"#,
            r#"{"env":{"ANTHROPIC_MODEL":"extracted","ANTHROPIC_BASE_URL":"https://x"},"includeCoAuthoredBy":false}"#,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "mine", "已有键不覆盖");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://x", "缺失键补上");
        assert_eq!(v["includeCoAuthoredBy"], serde_json::json!(false));
        // codex（TOML）：同样只补缺失。
        let merged_toml = merge_extracted_snippet(
            App::Codex,
            "[mcp_servers.a]\ncommand = \"x\"",
            "[mcp_servers.a]\ncommand = \"overwrite\"\n[mcp_servers.b]\ncommand = \"y\"",
        )
        .unwrap();
        assert!(merged_toml.contains("command = \"x\""), "已有 server 保留");
        assert!(merged_toml.contains("[mcp_servers.b]"), "缺失 server 补上");
    }
}
