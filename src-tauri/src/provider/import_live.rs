//! 「从 live 配置文件导入」：live → 待导入条目 → Provider 反向解析。
//!
//! opencode 的「从配置文件导入」泛化到 claude / codex / gemini / grok
//! （ADR-0012）。单激活应用（claude / codex / gemini / grok）的 live 是
//! **单份配置**（settings.json / config.toml / .env + settings.json /
//! config.toml），**一份 live → 至多 1 条条目**（0 条 = 文件缺失 / 空 / 无
//! 可识别身份内容），与 opencode 的 `provider.<key>` map（多条共存）本质不同
//! ——multiplicity 由 [`App::live_import_entries`] 的接口形状表达（单激活
//! 0..1 条、opencode 0..N 条），preview 与 import 从同一份条目推导（
//! [`preview_from_live_texts`] / [`import_from_live_texts`]，落库都走导入
//! seam）。
//!
//! 拆分规则 = 各应用写盘的反向镜像（受控字段表是单一事实来源，见 ADR-0005 /
//! ADR-0010）：
//! - **受控字段 → Provider.settings_config**（写盘时供应商接管的部分）。
//! - **可共享键 → snippet_candidates**（跨供应商共享候选，导入后提示「提取为
//!   通用片段」，T6）。「这个 app 哪些键可共享」每个 app 只声明一次（本模块
//!   「可共享键：单一决策源」一节的 `*_shareable_*`）：预览候选名册与提取内容
//!   （`*_extract_snippet`）都从同一份名册派生——加 / 改键规则只改名册，预览
//!   承诺与提取交付不会漂移（等价性由裁决测试
//!   `snippet_candidates_agree_with_extraction` 钉住）。
//!
//! 去重统一走 [`crate::provider::import`] 的 store 层 seam（冲突键策略作参数）：
//! 单激活应用按 **name**（`meta` 无 opencode 的 liveKey，`live_opencode::
//! meta_live_key` 是 opencode 专属）——同 app 同 name → 更新 name /
//! settings_config（保留 id / 展示字段 / meta），否则新建；opencode 按
//! **liveKey**——同 (app, liveKey) → 更新 name / settings_config / meta，
//! 否则新建。name = live 里 base_url 的注册域（去常见 TLD，用户认得出是哪个
//! 供应商），无 base_url → 应用名（"Claude" 等，数据非 i18n）。
//!
//! 所有转换函数是纯函数（测试接缝）：live 文本 → 快照 / 条目，不碰文件系统 /
//! DB；文件 IO 在命令层薄壳（`commands::live_import`），落库（seam）在本模块
//! （[`import_from_live_texts`]）。

use std::collections::HashMap;

use serde_json::Value;
use toml_edit::DocumentMut;

use crate::db::Store;
use crate::error::AppResult;
use crate::model::{App, Provider, ProviderCategory};
use crate::provider::import::{self, ImportKeyStrategy};
use crate::provider::live::{self, CONTROLLED_FIELDS};
use crate::provider::live_codex::CODEX_CONTROLLED_FIELDS;
use crate::provider::live_grok::{CC_ONE_PROFILE, MODELS_TABLE, MODEL_TABLE};
use crate::provider::live_opencode;
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

// ---------------- 统一导入条目（multiplicity 进 seam）----------------
//
// 「读 live → 0..N 条待导入条目」的统一中间形状：preview 与 import 从同一份
// 条目列表推导（此前单激活走 Option<快照>、opencode 走 provider.<key> map 各
// 解析一遍，预览承诺与导入交付靠两份代码人眼同步）。条目键的语义按 mode 分：
// 单激活 = name（Name 策略去重）、附加模式 = 配置文件原 key（LiveKey 策略）。

/// 一条「待导入条目」（单激活 0..1 条、opencode 0..N 条——条目数量即 mode 的
/// multiplicity，见 [`App::live_import_entries`]）。
#[derive(Debug, Clone)]
pub struct LiveImportEntry {
    /// 去重键：单激活 = name（Name 策略）、opencode = 配置文件原 key（LiveKey
    /// 策略）。预览「新建 vs 更新」与导入冲突规划用同一把键。
    pub key: String,
    /// 显示名（opencode = entry.name 优先、缺失/空串回退 key；单激活 =
    /// base_url 注册域或应用名）。
    pub name: String,
    /// 名字是否由 base_url 的注册域推导（单激活）；opencode 的名字来自
    /// entry.name / key，恒 false。前端理由行只在该标志为 true 时展示
    /// 「名取自 <url>」。
    pub name_derived_from_url: bool,
    /// base_url（预览展示；无 → 空串）。
    pub base_url: String,
    /// settings_config / entry 是否携带凭据（预览「含密钥」徽标；密钥值绝不
    /// 跨边界）。
    pub has_secret: bool,
    /// 可共享键候选（单激活应用导入后可提取为通用片段；opencode 无片段概念
    /// → 空）。
    pub snippet_candidates: Vec<String>,
    /// settingsConfig 文本（opencode = entry 子树 JSON）。
    pub settings_config: String,
    /// meta 文本（opencode = liveKey + liveManaged=true；单激活 = `"{}"`——
    /// Name 策略不更新 meta）。
    pub meta: String,
}

impl LiveImportEntry {
    /// 单激活快照 → 统一条目：key = name（Name 策略的去重键）、meta 空。
    pub(crate) fn from_snapshot(snap: LiveImportSnapshot) -> Self {
        LiveImportEntry {
            key: snap.name.clone(),
            name: snap.name,
            name_derived_from_url: !snap.base_url.is_empty(),
            base_url: snap.base_url,
            has_secret: snap.has_secret,
            snippet_candidates: snap.snippet_candidates,
            settings_config: snap.settings_config,
            meta: "{}".into(),
        }
    }

    /// 条目 → 待落库 Provider（纯函数）：id 空 → `save_provider` 生成；展示
    /// 字段空白，用户可在 UI 补。
    fn into_provider(self, app: App) -> Provider {
        Provider {
            id: String::new(),
            name: self.name,
            website_url: String::new(),
            category: ProviderCategory::Custom,
            app,
            icon: String::new(),
            icon_color: String::new(),
            sort_index: 0,
            notes: String::new(),
            settings_config: self.settings_config,
            meta: self.meta,
            updated_at: String::new(),
        }
    }
}

/// opencode.json 文本 → 0..N 条待导入条目（附加模式：`provider.<key>` map 里
/// 多供应商共存，一条 entry 一条目）。meta 记 liveKey = 原 key + liveManaged =
/// true（反复导入按 liveKey 去重）。
pub fn opencode_live_entries(live_text: &str) -> AppResult<Vec<LiveImportEntry>> {
    let mut entries = Vec::new();
    for (key, entry) in live_opencode::provider_entries(live_text) {
        // 行内显示名与预览同一推导（单一事实来源）：entry.name 非空优先，
        // 缺失或空串 → key。
        entries.push(LiveImportEntry {
            key: key.clone(),
            name: live_opencode::entry_display_name(&entry, &key),
            name_derived_from_url: false,
            base_url: entry
                .pointer("/options/baseURL")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            has_secret: secret_in_entry(&entry),
            snippet_candidates: vec![],
            settings_config: serde_json::to_string(&entry)?,
            meta: live_opencode::with_meta_live_state("", &key, true)?,
        });
    }
    Ok(entries)
}

/// entry 里是否携带凭据（只出布尔，不回取密钥值）：options.apiKey 非空，或
/// options.headers 任一值非空（headers 可携带 Authorization 等认证头）。
fn secret_in_entry(entry: &Value) -> bool {
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

/// 「从 live 配置导入」落库（可测，不碰文件系统——texts 由命令壳按
/// [`App::live_paths`] 顺序读入；缺失文件读为空串 = 0 条）：条目 → Provider →
/// 导入 seam。冲突键策略按 mode 分派（附加模式 = LiveKey、单激活 = Name——
/// 分派走 `is_additive_mode`，禁止 app 判等散落）。
/// `name_overrides` = 预览列表行内改名（key → name；单激活 key == name，同一
/// 覆盖语义）。返回写入条数。
pub fn import_from_live_texts(
    store: &Store,
    app: App,
    texts: &[String],
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let entries = app.live_import_entries(texts)?;
    if entries.is_empty() {
        return Ok(0);
    }
    let strategy = if app.is_additive_mode() {
        ImportKeyStrategy::LiveKey
    } else {
        ImportKeyStrategy::Name
    };
    let incoming = entries
        .into_iter()
        .map(|mut entry| {
            if let Some(name) = name_overrides.get(&entry.key) {
                entry.name = name.clone();
            }
            entry.into_provider(app)
        })
        .collect::<Vec<_>>();
    let report = import::import_providers(store, &incoming, strategy)?;
    Ok(report.imported)
}

/// 一条预览行：条目 + is_new（去重键是否已在 DB）。
#[derive(Debug, Clone)]
pub struct LiveImportPreviewRow {
    /// 将导入的条目（与 [`import_from_live_texts`] 同源推导）。
    pub entry: LiveImportEntry,
    /// DB 无此去重键 → 新建；有 → 更新。
    pub is_new: bool,
}

/// 「从 live 配置导入」预览核心（可测，不碰文件系统/DB 之外的任何东西——
/// store 只读现有列表判 is_new）：texts → 0..N 条预览行。is_new 的去重键集合
/// 与导入的冲突键策略同一分派（附加模式 = meta.liveKey 集合、单激活 = name
/// 集合）——预览不承诺导入交付不了的判定。
pub fn preview_from_live_texts(
    store: &Store,
    app: App,
    texts: &[String],
) -> AppResult<Vec<LiveImportPreviewRow>> {
    let entries = app.live_import_entries(texts)?;
    let existing_keys: std::collections::HashSet<String> = store
        .list_providers_for(app)?
        .into_iter()
        .filter_map(|p| {
            if app.is_additive_mode() {
                live_opencode::meta_live_key(&p.meta)
            } else {
                Some(p.name)
            }
        })
        .collect();
    Ok(entries
        .into_iter()
        .map(|entry| {
            let is_new = !existing_keys.contains(&entry.key);
            LiveImportPreviewRow { entry, is_new }
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::testutil::mem;

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

    // ---------------- opencode 反向导入（LiveKey 策略，seam 落库）----------------

    /// 一份带两个 provider（一个带 name、一个不带）+ 顶层用户字段的 opencode.json。
    /// `pub(crate)`：命令层预览测试与导入测试共用同一夹具（单一事实来源，
    /// 不各留一份漂移）。
    pub(crate) fn opencode_live_json() -> &'static str {
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
    fn import_opencode_creates_providers_with_live_key_and_managed_flag() {
        let s = mem();
        let n = import_from_live_texts(
            &s,
            App::OpenCode,
            &[opencode_live_json().to_string()],
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
        let sc: Value = serde_json::from_str(&ds.settings_config).unwrap();
        assert_eq!(sc["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(sc["options"]["baseURL"], "https://api.deepseek.com");
    }

    /// 反复 import 同一文件 → 按 liveKey 匹配更新，不产生重复。
    #[test]
    fn import_opencode_updates_existing_same_live_key_no_duplicate() {
        let s = mem();
        let texts = [opencode_live_json().to_string()];
        import_from_live_texts(&s, App::OpenCode, &texts, &HashMap::new()).unwrap();
        let n = import_from_live_texts(&s, App::OpenCode, &texts, &HashMap::new()).unwrap();
        assert_eq!(n, 2, "第二次仍处理 2 条（按 liveKey 更新）");
        assert_eq!(
            s.list_providers_for(App::OpenCode).unwrap().len(),
            2,
            "不产生重复"
        );
    }

    /// 行内改名覆盖：nameOverrides（key → name）优先于 entry.name / key。
    #[test]
    fn import_opencode_respects_name_overrides() {
        let s = mem();
        let overrides = HashMap::from([("deepseek".to_string(), "DS 直连".to_string())]);
        let n = import_from_live_texts(
            &s,
            App::OpenCode,
            &[opencode_live_json().to_string()],
            &overrides,
        )
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
    fn import_opencode_empty_providers_section_is_zero() {
        let s = mem();
        let n = import_from_live_texts(
            &s,
            App::OpenCode,
            &[r#"{"model":"x"}"#.to_string()],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 0);
        assert!(s.list_providers_for(App::OpenCode).unwrap().is_empty());
    }

    // ---------------- 单激活应用导入（Name 策略，seam 落库）----------------

    /// claude live → 条目 → 落库：按 name（base_url 注册域）新建 Provider。
    #[test]
    fn import_from_live_creates_provider_with_registry_domain_name() {
        let s = mem();
        let n = import_from_live_texts(
            &s,
            App::Claude,
            &[r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"kimi"}}"#.to_string()],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 1);
        let providers = s.list_providers_for(App::Claude).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(
            providers[0].name, "moonshot",
            "注册域去 TLD（host_of 规则）"
        );
        let sc: Value = serde_json::from_str(&providers[0].settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "kimi");
    }

    /// 反复导入同 name → 更新不重复（单激活按 name 去重）。
    #[test]
    fn import_from_live_dedupes_by_name() {
        let s = mem();
        let texts = [r#"{"env":{"ANTHROPIC_MODEL":"m1"}}"#.to_string()];
        import_from_live_texts(&s, App::Claude, &texts, &HashMap::new()).unwrap();
        import_from_live_texts(&s, App::Claude, &texts, &HashMap::new()).unwrap();
        assert_eq!(
            s.list_providers_for(App::Claude).unwrap().len(),
            1,
            "同 name 不产生重复"
        );
    }

    /// Name 策略的更新语义（store 层）：同 name 第二次导入 → 更新
    /// settings_config，保留已有行的 id / 展示字段 / meta。
    #[test]
    fn import_from_live_updates_settings_keeps_id_and_display_fields() {
        let s = mem();
        let first = r#"{"env":{"ANTHROPIC_MODEL":"m1","ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic"}}"#;
        import_from_live_texts(&s, App::Claude, &[first.to_string()], &HashMap::new()).unwrap();
        let row = &s.list_providers_for(App::Claude).unwrap()[0];
        let id = row.id.clone();
        let meta = row.meta.clone();
        assert_eq!(row.name, "moonshot", "注册域作 name");

        let second = r#"{"env":{"ANTHROPIC_MODEL":"m2","ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic"}}"#;
        let n = import_from_live_texts(&s, App::Claude, &[second.to_string()], &HashMap::new())
            .unwrap();
        assert_eq!(n, 1);
        let rows = s.list_providers_for(App::Claude).unwrap();
        assert_eq!(rows.len(), 1, "同 name 不新建");
        assert_eq!(rows[0].id, id, "保留已有 id");
        assert_eq!(rows[0].meta, meta, "meta 保留（Name 策略不更新 meta）");
        assert_eq!(rows[0].name, "moonshot", "name 不变（同 name 更新）");
        let sc: Value = serde_json::from_str(&rows[0].settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "m2", "settings_config 更新");
    }

    // ---------------- 统一预览（与导入同一条目推导）----------------

    /// 预览提取 name / endpoint / 密钥布尔 / 去重键：带 name 用 name，缺 name
    /// 用 key；is_new 按去重键（liveKey）集合判定。条目与导入同源（同一份
    /// live_import_entries 产物）。
    #[test]
    fn preview_lists_entries_with_name_endpoint_and_secret() {
        let s = mem();
        let rows = preview_from_live_texts(&s, App::OpenCode, &[opencode_live_json().to_string()])
            .unwrap();
        assert_eq!(rows.len(), 2, "字母序：deepseek 先于 kimi");
        let ds = &rows[0].entry;
        assert_eq!(ds.key, "deepseek");
        assert_eq!(ds.name, "DeepSeek", "entry.name 作显示名");
        assert_eq!(ds.base_url, "https://api.deepseek.com");
        assert!(ds.has_secret, "options.apiKey 非空 → 含密钥");
        assert!(rows[0].is_new, "DB 无此 liveKey → 新建");
        let kimi = &rows[1].entry;
        assert_eq!(kimi.key, "kimi");
        assert_eq!(kimi.name, "kimi", "无 name → key 作显示名");
        assert_eq!(kimi.base_url, "https://api.moonshot.cn");
        assert!(!kimi.has_secret, "无 apiKey → 不含密钥");
    }

    /// 「新建 vs 更新」判定与导入一致：existing_keys 按 liveKey 集合判定。
    #[test]
    fn preview_classifies_new_vs_update() {
        let s = mem();
        // 种入一条已托管的 deepseek（meta.liveKey = deepseek）。
        s.save_provider(crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "",
            "DeepSeek",
            r#"{"npm":"@ai-sdk/openai-compatible"}"#,
            r#"{"liveKey":"deepseek","liveManaged":true}"#,
        ))
        .unwrap();
        let rows = preview_from_live_texts(&s, App::OpenCode, &[opencode_live_json().to_string()])
            .unwrap();
        assert!(!rows[0].is_new, "已有同 liveKey → 更新");
        assert!(rows[1].is_new, "无此 liveKey → 新建");
    }

    /// 空 name（`"name": ""`）导入与预览同判：回退 key（与导入共用
    /// entry_display_name，#67）。
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
        let s = mem();
        let rows = preview_from_live_texts(&s, App::OpenCode, &[live.to_string()]).unwrap();
        assert_eq!(rows[0].entry.name, "blank", "预览：空 name → key");
        import_from_live_texts(&s, App::OpenCode, &[live.to_string()], &HashMap::new()).unwrap();
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        assert_eq!(
            providers[0].name, "blank",
            "导入与预览同一显示名（空 name → key，不再存空串）"
        );
    }

    /// headers 也能携带凭据（Authorization 等）→ 计入 has_secret；空值不算。
    #[test]
    fn preview_detects_headers_secret() {
        let s = mem();
        let live = r#"{
          "provider": {
            "h1": { "options": { "headers": { "Authorization": "Bearer abc" } } },
            "h2": { "options": { "headers": { "X-Empty": "" } } },
            "h3": { "options": { "apiKey": "" } }
          }
        }"#;
        let rows = preview_from_live_texts(&s, App::OpenCode, &[live.to_string()]).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].entry.has_secret, "headers 非空值 → 含密钥");
        assert!(!rows[1].entry.has_secret, "headers 空值 → 不含");
        assert!(!rows[2].entry.has_secret, "apiKey 空串 → 不含");
    }

    /// 无 provider 段 / 损坏 JSON5 / 非对象根 → 空（与导入「静默 0 条」一致，
    /// preview 与 import 语义不得分叉）。
    #[test]
    fn preview_empty_or_unparseable_live_is_empty() {
        let s = mem();
        for live in [r#"{"model":"x"}"#, "{bad", "[1,2]", ""] {
            let rows = preview_from_live_texts(&s, App::OpenCode, &[live.to_string()]).unwrap();
            assert!(rows.is_empty(), "输入 {live:?} 应 → 空");
        }
    }

    /// 单激活应用的预览行：0..1 条、key == name（Name 策略去重键）、名字由
    /// base_url 推导时 name_derived_from_url = true、is_new 按 name 集合判定。
    #[test]
    fn preview_single_activate_yields_at_most_one_row() {
        let s = mem();
        let live = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"kimi"}}"#;
        let rows = preview_from_live_texts(&s, App::Claude, &[live.to_string()]).unwrap();
        assert_eq!(rows.len(), 1, "单激活一份 live → 至多 1 条");
        assert_eq!(rows[0].entry.key, "moonshot", "去重键 = name");
        assert!(rows[0].entry.name_derived_from_url, "名字取自 base_url");
        assert!(rows[0].is_new, "DB 无同名 → 新建");
        // 同名落库后再预览 → is_new = false（与导入去重同一把键）。
        import_from_live_texts(&s, App::Claude, &[live.to_string()], &HashMap::new()).unwrap();
        let rows = preview_from_live_texts(&s, App::Claude, &[live.to_string()]).unwrap();
        assert!(!rows[0].is_new, "同 name 已存在 → 更新");
        // 无受控内容 → 0 条（与导入同判）。
        assert!(
            preview_from_live_texts(&s, App::Claude, &["{}".to_string()])
                .unwrap()
                .is_empty()
        );
    }
}
