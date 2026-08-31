//! 「读 live 长什么样」：live 配置文本 → 导入快照 / 可共享片段内容的纯函数半。
//!
//! 「从 live 配置文件导入」（父模块 [`super`]）分两半，本模块是**解析半**：
//! 单激活应用的一份 live → 至多 1 个 [`LiveImportSnapshot`]（`None` = 文件缺失
//! / 空 / 无可识别身份内容），以及导入后「提取为通用片段」的内容
//! （`*_extract_snippet`）。条目化（multiplicity）与落库在父模块：快照经
//! [`super::LiveImportEntry::from_snapshot`] 变条目，preview 与 import 从同一
//! 份条目推导（[`super::preview_from_live_texts`] /
//! [`super::import_from_live_texts`]）。opencode 的 `provider.<key>` map（多条
//! 共存）不走快照，条目化在父模块 [`super::opencode_live_entries`]。
//!
//! 解析规则 = 各应用写盘的反向镜像（受控字段表是单一事实来源，见 ADR-0005 /
//! ADR-0010）：
//! - **受控字段 → Provider.settings_config**（写盘时供应商接管的部分）。
//! - **可共享键 → snippet_candidates**（跨供应商共享候选，导入后提示「提取为
//!   通用片段」，T6）。「这个 app 哪些键可共享」每个 app 只声明一次（本模块
//!   「可共享键：单一决策源」一节的 `*_shareable_*`）：预览候选名册与提取内容
//!   （`*_extract_snippet`）都从同一份名册派生——加 / 改键规则只改名册，预览
//!   承诺与提取交付不会漂移（等价性由裁决测试
//!   `snippet_candidates_agree_with_extraction` 钉住）。
//!
//! 所有函数是纯函数（测试接缝）：live 文本 → 快照 / 片段内容，不碰文件系统 /
//! DB；文件 IO 在命令层薄壳（`commands::live_import`），落库（seam）在父模块。

use std::collections::HashMap;

use serde_json::Value;
use toml_edit::DocumentMut;

use crate::provider::live::{self, CONTROLLED_FIELDS};
use crate::provider::live_codex::CODEX_CONTROLLED_FIELDS;
use crate::provider::live_grok::{CC_ONE_PROFILE, MODELS_TABLE, MODEL_TABLE};
use crate::provider::settings_codec::{
    build_codex_settings, build_gemini_settings, build_grok_settings, CODEX_AUTH_SECRET_KEY,
    GOOGLE_GEMINI_BASE_URL_ENV,
};
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
    /// 可共享键候选（导入后提示「提取为通用片段」，T6）。与
    /// `*_extract_snippet` 的提取内容同源派生（见本模块「可共享键：单一决策
    /// 源」）：候选 == 提取交付的键（裁决测试
    /// `snippet_candidates_agree_with_extraction` 钉住）。
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

// ---------------- 可共享键：单一决策源（预览候选 ⇄ 提取共用）----------------

// 「这个 app 的 live 里哪些键可共享为通用片段」每个 app 只声明一次（下面四组
// `*_shareable_*` 函数）：快照的候选名册（`*_live_to_snapshot` 的
// `snippet_candidates`）与提取内容（`*_extract_snippet`）都从它派生——候选 =
// 名册的键，提取 = 名册键对应的值。加 / 改某 app 的可共享键规则只改它那组
// 函数，预览承诺与提取交付不会漂移；等价性由裁决测试
// `snippet_candidates_agree_with_extraction` 钉住。
//
// 名册粒度跟着片段合并层（ADR-0010）走，是各 app 片段语义的必然结果而非
// 任意选择：
// - claude：片段在 settings_config 层合并，合并域 = env 子键 + 受控非 env
//   顶层开关（`snippet::MergeDomain::ControlledFields`）→ 名册 = env 子键 +
//   顶层开关；
// - gemini：片段在 settings_config 层合并，合并域 = env 子键 + settings.json
//   顶层整体（`snippet::MergeDomain::WholeTopLevel`，机制能承载顶层键）——但
//   **名册仍只列 env 子键**：这是 ADR-0010 的名册决策（Gemini 片段 = JSON env
//   对象），不是合并机制的约束；顶层键经片段校验（`validate_gemini_snippet`）
//   拒绝，列了只会让「提取」提示交付不了的东西；
// - codex / grok：片段在写盘层整表补缺失（`live::fill_missing_table`）→
//   名册 = config.toml 非受控顶层键。

/// claude 可共享 env 子键值对（决策源 env 半）：非敏感键（凭据永不共享）。
fn claude_shareable_env(env: &serde_json::Map<String, Value>) -> Vec<(String, Value)> {
    env.iter()
        .filter(|(k, _)| !is_sensitive_config_key(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// claude 可共享顶层键值对（决策源顶层半）：受控非 env 键（`includeCoAuthoredBy`
/// 等开关；live 里存在才列——片段合并层只认 [`CONTROLLED_FIELDS`]，其余顶层键
/// 合并时零效果，列了只会让「提取」提示空欢喜）。
fn claude_shareable_top(live: &serde_json::Map<String, Value>) -> Vec<(String, Value)> {
    CONTROLLED_FIELDS
        .iter()
        .filter(|k| **k != "env")
        .filter_map(|k| live.get(*k).map(|v| ((*k).to_string(), v.clone())))
        .collect()
}

/// gemini 可共享 env 键值对（决策源）：非敏感、非端点键
/// （[`GOOGLE_GEMINI_BASE_URL_ENV`]——端点决定凭据发往何处，归供应商）、值
/// 非空（set 校验拒空值，提取与手动保存同判——候选同判：预览不承诺提取交付
/// 不了的键）。settings.json 的键不在名册：这是 ADR-0010 的名册决策（Gemini
/// 片段 = JSON env 对象）——合并域（顶层整体）虽能承载顶层键，名册放宽另行
/// 决策。
fn gemini_shareable_env(env: &HashMap<String, String>) -> Vec<(String, String)> {
    env.iter()
        .filter(|(k, v)| {
            !is_sensitive_config_key(k)
                && k.as_str() != GOOGLE_GEMINI_BASE_URL_ENV
                && !v.trim().is_empty()
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// codex 可共享顶层项（决策源）：config.toml 非受控顶层键（非
/// [`CODEX_CONTROLLED_FIELDS`]，`mcp_servers` / `web_search` 等用户手动共享
/// 配置）。返回 (键, 项) 引用——预览候选拿键名，提取拿项整表搬运。
fn codex_shareable(doc: &DocumentMut) -> Vec<(&str, &toml_edit::Item)> {
    doc.as_table()
        .iter()
        .filter(|(k, _)| !CODEX_CONTROLLED_FIELDS.contains(k))
        .collect()
}

/// grok 可共享顶层项（决策源）：顶层非 model / models 项（表名常量归
/// `live_grok`，改表名只改那边）。`mcp_servers` 等非受控共享键；`[model.*]`
/// 各 profile（含用户自建）整表归身份侧，不进名册。
fn grok_shareable(doc: &DocumentMut) -> Vec<(&str, &toml_edit::Item)> {
    doc.as_table()
        .iter()
        .filter(|(k, _)| *k != MODEL_TABLE && *k != MODELS_TABLE)
        .collect()
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
    // 片段候选 = 可共享键名册的键（与提取同一决策源）：env 非敏感子键 +
    // 受控非 env 顶层开关。settings 是 live 的受控子集，但判定一律以 live
    // 原文（map）为准——与提取同源，不留「两边各自看着等价」的默契。
    let mut snippet_candidates: Vec<String> = map
        .get("env")
        .and_then(|e| e.as_object())
        .map(claude_shareable_env)
        .unwrap_or_default()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    snippet_candidates.extend(claude_shareable_top(map).into_iter().map(|(k, _)| k));
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Claude".to_string()),
        settings_config: serde_json::to_string_pretty(&settings).ok()?,
        base_url,
        has_secret,
        snippet_candidates,
    })
}

/// codex：`config.toml` + `auth.json` → 快照。settings_config 由 codec 的
/// build 半向构造（`{"auth":{OPENAI_API_KEY}, "config":"<只含
/// CODEX_CONTROLLED_FIELDS 键的 TOML>"}`）；auth 只取密钥键（trim 非空才有，
/// 其余登录态字段不导）。无可识别受控内容 → `None`。
pub fn codex_live_to_snapshot(config_toml: &str, auth_json: &str) -> Option<LiveImportSnapshot> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    // 受控键子集 TOML（写盘 `merge_codex_config` 的反向）。
    let mut out = DocumentMut::new();
    for key in CODEX_CONTROLLED_FIELDS {
        if let Some(item) = doc.get(key) {
            out.insert(key, item.clone());
        }
    }
    // auth：只取密钥键（trim 后非空才有；值原样保留）。
    let auth_key = if auth_json.trim().is_empty() {
        None
    } else {
        serde_json::from_str::<Value>(auth_json)
            .ok()?
            .as_object()?
            .get(CODEX_AUTH_SECRET_KEY)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    if out.as_table().is_empty() && auth_key.is_none() {
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
    let has_secret = auth_key.is_some();
    // 片段候选 = 可共享键名册的键（与提取同一决策源）：非受控顶层键。
    let snippet_candidates = codex_shareable(&doc)
        .iter()
        .map(|(k, _)| (*k).to_string())
        .collect();
    let settings_config = build_codex_settings(auth_key.as_deref(), &out.to_string());
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Codex".to_string()),
        settings_config,
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
        .get(GOOGLE_GEMINI_BASE_URL_ENV)
        .map(String::as_str)
        .unwrap_or("")
        .to_string();
    let has_secret = env.keys().any(|k| is_sensitive_config_key(k));
    // 片段候选 = 可共享键名册的键（与提取同一决策源）：env 非敏感非端点非空值
    // 键——settings.json 的非受控键进片段零效果（提取只取 env），不列入。
    let snippet_candidates = gemini_shareable_env(&env)
        .iter()
        .map(|(k, _)| k.clone())
        .collect();
    // settings_config 形状（env + config 的包装）归 codec build 半向。
    let settings_config = build_gemini_settings(env, config);
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Gemini".to_string()),
        settings_config,
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
    // cc-one profile 定位（表名 / profile 名常量归 live_grok，写盘与反向解析
    // 共用同一份）。
    fn cc_one_profile(doc: &DocumentMut) -> Option<&toml_edit::Item> {
        doc.get(MODEL_TABLE).and_then(|t| t.get(CC_ONE_PROFILE))
    }
    let profile = cc_one_profile(&doc).cloned()?;
    // 只保留 [model."cc-one"] 块（用户其它 profile / mcp_servers 不进 settings_config）。
    // model 表标 implicit——只渲染 [model."cc-one"]、不产出孤立的 [model] 头（与
    // 写盘 `merge_grok_config` 同一构造）。
    let mut out = DocumentMut::new();
    let mut model = toml_edit::Table::new();
    model.insert(CC_ONE_PROFILE, profile);
    model.set_implicit(true);
    out.as_table_mut()
        .insert(MODEL_TABLE, toml_edit::Item::Table(model));
    let base_url = cc_one_profile(&doc)
        .and_then(|t| t.get("base_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let has_secret = cc_one_profile(&doc)
        .and_then(|t| t.get("api_key"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    // 片段候选 = 可共享键名册的键（与提取同一决策源）：顶层非 model / models 项。
    let snippet_candidates = grok_shareable(&doc)
        .iter()
        .map(|(k, _)| (*k).to_string())
        .collect();
    let settings_config = build_grok_settings(&out.to_string());
    Some(LiveImportSnapshot {
        name: name_from_base_url(&base_url).unwrap_or_else(|| "Grok".to_string()),
        settings_config,
        base_url,
        has_secret,
        snippet_candidates,
    })
}

/// 从 live 提取「可共享键」为片段内容（T6，ADR-0012 consequence L30——导入后
/// 检测非身份共享键、非静默提示「提取为通用片段」）。各 app 提取什么不由本组
/// 函数决定：内容 = 「可共享键单一决策源」名册对应的值——claude = env 非敏感
/// 子键 + 受控顶层开关的值；gemini = env 名册的值；codex / grok = 非受控顶层
/// 项整表搬运。与快照候选名册同源派生：加 / 改键规则只改 `*_shareable_*`，
/// 预览承诺与提取交付不会漂移。
///
/// 无可提取（名册为空）→ `None`。
pub fn claude_extract_snippet(live: &str) -> Option<String> {
    let obj = live::parse_live_or_empty(live).ok()?;
    let map = obj.as_object()?;
    // 提取内容 = 名册键对应的值：env 子键级挑值 + 顶层开关整值搬运。
    let mut out: serde_json::Map<String, Value> = serde_json::Map::new();
    let env_out: serde_json::Map<String, Value> = map
        .get("env")
        .and_then(|e| e.as_object())
        .map(claude_shareable_env)
        .unwrap_or_default()
        .into_iter()
        .collect();
    if !env_out.is_empty() {
        out.insert("env".into(), Value::Object(env_out));
    }
    for (k, v) in claude_shareable_top(map) {
        out.insert(k, v);
    }
    if out.is_empty() {
        None
    } else {
        Some(serde_json::to_string_pretty(&out).ok()?)
    }
}

pub fn gemini_extract_snippet(env_text: &str) -> Option<String> {
    let env = parse_env_file(env_text);
    // 提取内容 = env 名册的值（含空值排除——set 校验拒空值，#60）。
    let env_out: serde_json::Map<String, Value> = gemini_shareable_env(&env)
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    if env_out.is_empty() {
        None
    } else {
        Some(serde_json::json!({ "env": env_out }).to_string())
    }
}

pub fn codex_extract_snippet(config_toml: &str) -> Option<String> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    // 提取内容 = 名册顶层项整表搬运。
    let mut out = DocumentMut::new();
    for (k, item) in codex_shareable(&doc) {
        out.insert(k, item.clone());
    }
    if out.as_table().is_empty() {
        None
    } else {
        Some(out.to_string())
    }
}

pub fn grok_extract_snippet(config_toml: &str) -> Option<String> {
    let doc = live::parse_toml_or_empty(config_toml, "live config.toml").ok()?;
    // 提取内容 = 名册顶层项整表搬运。
    let mut out = DocumentMut::new();
    for (k, item) in grok_shareable(&doc) {
        out.insert(k, item.clone());
    }
    if out.as_table().is_empty() {
        None
    } else {
        Some(out.to_string())
    }
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

    /// 裁决性测试（预览 ⇄ 提取不变量）：快照承诺的候选名册 == 提取产物交付的
    /// 键（四个 app 各一例）。此前「候选 ⊆ 提取」只有散文守着、两份实现各算
    /// 一遍；现在两者同源派生（`*_shareable_*` 名册），此测试把「预览不空欢喜」
    /// 钉成红绿灯。比较粒度 = 各 app 的名册粒度：claude / gemini 展平到 env
    /// 子键 + 顶层开关（前端 `snippetCoveredKeys` 对 JSON 片段正是按「顶层键 +
    /// env 内键」收键，同一契约）；codex / grok 即 TOML 顶层键。
    #[test]
    fn snippet_candidates_agree_with_extraction() {
        fn sorted(keys: &[String]) -> Vec<String> {
            let mut v = keys.to_vec();
            v.sort();
            v
        }

        // claude：候选 == 提取交付（env 非敏感子键 + 顶层开关）；凭据键与非受
        // 控顶层键两边都不出现。
        let claude_live = r#"{
            "env": {"ANTHROPIC_MODEL": "kimi", "ANTHROPIC_AUTH_TOKEN": "sk-x", "ANTHROPIC_BASE_URL": "https://x"},
            "includeCoAuthoredBy": false,
            "permissions": {"allow": ["Bash"]}
        }"#;
        let claude_snap = claude_live_to_snapshot(claude_live).unwrap();
        let claude_out: Value =
            serde_json::from_str(&claude_extract_snippet(claude_live).unwrap()).unwrap();
        let mut claude_delivered: Vec<String> = claude_out["env"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        claude_delivered.extend(
            claude_out
                .as_object()
                .unwrap()
                .keys()
                .filter(|k| *k != "env")
                .cloned(),
        );
        assert_eq!(
            sorted(&claude_snap.snippet_candidates),
            sorted(&claude_delivered),
            "claude：候选 == 提取交付（env 子键 + 顶层开关）"
        );
        assert!(
            !claude_delivered.iter().any(|k| k == "ANTHROPIC_AUTH_TOKEN"),
            "凭据键既不承诺也不交付"
        );
        assert!(
            !claude_delivered.iter().any(|k| k == "permissions"),
            "非受控顶层键两边都不出现"
        );

        // gemini：候选 == 提取交付的 env 子键；端点键与空值行两边都不出现
        // （空值排除此前只在提取侧——候选曾承诺提取交付不了的键，已对齐）。
        let gemini_env = "GOOGLE_GEMINI_BASE_URL=https://x.dev\nGEMINI_API_KEY=sk-x\nGEMINI_MODEL=m\nGEMINI_EMPTY=\n";
        let gemini_snap = gemini_live_to_snapshot(gemini_env, "").unwrap();
        let gemini_out: Value =
            serde_json::from_str(&gemini_extract_snippet(gemini_env).unwrap()).unwrap();
        let gemini_delivered: Vec<String> = gemini_out["env"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            sorted(&gemini_snap.snippet_candidates),
            sorted(&gemini_delivered),
            "gemini：候选 == 提取交付（env 子键）"
        );
        assert!(
            !gemini_delivered.contains(&"GEMINI_EMPTY".to_string()),
            "空值行不承诺不交付（#60：set 校验拒空值）"
        );
        assert!(
            !gemini_delivered.contains(&"GOOGLE_GEMINI_BASE_URL".to_string()),
            "端点键不承诺不交付"
        );

        // codex / grok：候选 == 提取产物 TOML 顶层键。
        let codex_config = "model = \"gpt-5\"\n\n[mcp_servers.github]\ncommand = \"npx\"\n\n[web_search]\nenabled = true\n";
        let codex_snap = codex_live_to_snapshot(codex_config, "{}").unwrap();
        let codex_out: DocumentMut = codex_extract_snippet(codex_config)
            .expect("有可提取")
            .parse()
            .unwrap();
        let codex_delivered: Vec<String> = codex_out
            .as_table()
            .iter()
            .map(|(k, _)| k.to_string())
            .collect();
        assert_eq!(
            sorted(&codex_snap.snippet_candidates),
            sorted(&codex_delivered),
            "codex：候选 == 提取交付（TOML 顶层键）"
        );
        assert_eq!(
            codex_delivered,
            vec!["mcp_servers".to_string(), "web_search".to_string()]
        );

        let grok_config = "[models]\ndefault = \"cc-one\"\n\n[model.cc-one]\nmodel = \"grok-4.5\"\n\n[mcp_servers.fs]\ncommand = \"npx\"\n";
        let grok_snap = grok_live_to_snapshot(grok_config).unwrap();
        let grok_out: DocumentMut = grok_extract_snippet(grok_config)
            .expect("有可提取")
            .parse()
            .unwrap();
        let grok_delivered: Vec<String> = grok_out
            .as_table()
            .iter()
            .map(|(k, _)| k.to_string())
            .collect();
        assert_eq!(
            sorted(&grok_snap.snippet_candidates),
            sorted(&grok_delivered),
            "grok：候选 == 提取交付（TOML 顶层键）"
        );
        assert_eq!(grok_delivered, vec!["mcp_servers".to_string()]);
    }
}
