//! 从 CC-Switch 导入供应商：转换纯函数 + 统一供应商展开 + 代理类跳过判定。
//!
//! CC-Switch 是同类工具，用户迁移到 cc one 时，已在 CC-Switch 配好的供应商
//! （端点 / key / 模型）存在它本机的 SQLite（`~/.cc-switch/cc-switch.db`）里。
//! 本模块把这些供应商**翻译**成 cc one 的 `Provider`，由命令层直接喂
//! [`crate::provider::import`] 的 store 层 seam 写库（AppId 策略）——不新造
//! 冲突逻辑，也不经导出文档序列化绕道。
//!
//! **纯函数是测试接缝**：[`convert_ccswitch_provider`] 吃进一个 CC-Switch 供应
//! 商（带 `app_type`），产出 [`ConvertOutcome`]；不碰数据库 / 网络 / 文件——读
//! 文件在命令层做完，把原始数据喂进来，测试直测输入 → 输出。
//!
//! **应用范围**：CC-Switch 配置侧 8 个应用，cc one 的 `App` 有 5 个。映射：
//! claude / codex / gemini / grokbuild / opencode → 对应 cc one `App`（全部
//! 转换并导入）；claude-desktop / openclaw / hermes → `Skipped(UnsupportedApp)`。
//!
//! **取消搁置**：grokbuild / opencode 曾因 cc one 无写盘而被搁置；如今 cc one
//! 已能管 Grok + OpenCode（写盘就绪），搁置前提不复存在——故本实现 5 应用全部
//! 落库，不再产出 `Shelved`。
//!
//! **代理类 / OAuth 跳过**：需本地代理或 OAuth 的供应商（GitHub Copilot、
//! Codex 登录态、Grok OAuth、Gemini Native、非 anthropic 协议中转）cc one
//! 架构上不做代理，搬过来用不了——跳过并记原因（[`SkipReason`]）。
//!
//! **统一供应商**（`settings` 表 `universal_providers`）：一个跨应用共享的聚合
//! 网关。cc one 每应用独立池、不做跨应用共享，故把它展开成 claude / codex /
//! gemini 各自的独立子 Provider（id 前缀 `universal-<app>-`），配置不丢。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use toml_edit::{DocumentMut, Item, Table};

use crate::error::{AppError, AppResult};
use crate::model::{App, Provider, ProviderCategory};
use rusqlite::{Connection, OptionalExtension};

// ── 输入类型（CC-Switch 侧，宽容反序列化）───────────────────────────────────

/// CC-Switch 的一条供应商（转换纯函数的输入）。字段从 CC-Switch DB 行 / 旧 JSON
/// 反序列化；`settings_config` / `meta` 已由命令层从 DB 的 TEXT 列解析为 JSON
/// [`Value`]。宽容：多余字段忽略，可选字段缺失归零。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchProvider {
    pub id: String,
    pub name: String,
    /// DB 行的 `app_type` 列（旧 JSON 里是外层 map 的 key，由命令层填入）。
    pub app_type: String,
    /// 已解析的 settings_config（各应用形状：claude/gemini = `{env}`、codex =
    /// `{auth, config}`、grokbuild = `{config}`、opencode = `{npm, options, models}`）。
    #[serde(default = "default_object_value")]
    pub settings_config: Value,
    #[serde(default)]
    pub website_url: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub icon_color: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i64>,
    #[serde(default)]
    pub notes: Option<String>,
    /// 已解析的 meta（含代理层字段 `providerType` / `apiFormat`，用于跳过判定；
    /// 原样保留进 cc one meta，不在导入层清洗——写盘自会剥非受控字段）。
    #[serde(default = "default_object_value")]
    pub meta: Value,
}

fn default_object_value() -> Value {
    Value::Object(Default::default())
}

/// CC-Switch 的统一供应商（`settings` 表 `universal_providers`）。一个跨应用
/// 共享的聚合网关，`apps` 只有 claude / codex / gemini 三个 bool（不涉及其它
/// 应用，见研究文档 §4.1）。模型字段不取——展开时用该应用的缺省模型（用户
/// 可在 cc one 内再调 / 「获取模型」拉取）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalProvider {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub provider_type: String,
    #[serde(default)]
    pub apps: UniversalApps,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub website_url: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub icon_color: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UniversalApps {
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub codex: bool,
    #[serde(default)]
    pub gemini: bool,
}

// ── 输出类型 ────────────────────────────────────────────────────────────────

/// 转换一条 CC-Switch 供应商的结果。取消搁置后只剩两态（见模块文档）。
#[derive(Debug, Clone, PartialEq)]
pub enum ConvertOutcome {
    /// 成功翻译成 cc one Provider，待写入。
    Imported(Provider),
    /// 跳过：需代理 / OAuth，或不支持的应用。
    Skipped { name: String, reason: SkipReason },
}

/// 跳过原因（跨 Rust→JS 边界，报告里展示给用户）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// `meta.apiFormat ∈ {openai_chat, openai_responses, gemini_native}`——需协议
    /// 转换代理，cc one 不做。
    NeedsProxy,
    /// `meta.providerType ∈ {github_copilot, codex_oauth, xai_oauth}`，或 Claude
    /// 端点命中 `githubcopilot.com` / `chatgpt.com/backend-api/codex`——需 OAuth。
    NeedsOAuth,
    /// claude-desktop / openclaw / hermes 等 cc one 不做的应用。
    UnsupportedApp,
}

/// CC-Switch 导入报告（跨 Rust→JS 边界，`u32` 计数避免 specta BigInt 问题）。
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CcSwitchImportReport {
    /// 实际写入数（来自导入 seam 的 AppId 策略）。
    pub imported: u32,
    /// merge 模式下 (app, id) 冲突跳过数。
    pub merge_skipped: u32,
    /// 代理 / OAuth / 不支持应用跳过明细。
    pub proxy_skipped: Vec<SkipDetail>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkipDetail {
    pub name: String,
    pub reason: SkipReason,
}

// ── 核心纯函数 ───────────────────────────────────────────────────────────────

/// 转换一条 CC-Switch 供应商为 cc one Provider（或跳过）。纯函数：不碰 DB / 文件。
///
/// 顺序：先 app 映射（不支持的直接 `UnsupportedApp`，不被代理类判定截胡），再
/// 代理类 / OAuth 判定，最后字段映射构造 Provider。`now_iso` 由调用方传入
/// （避免纯函数依赖系统时间，测试可控）。
pub fn convert_ccswitch_provider(p: &CcSwitchProvider, now_iso: &str) -> ConvertOutcome {
    let Some(app) = map_app(&p.app_type) else {
        return ConvertOutcome::Skipped {
            name: p.name.clone(),
            reason: SkipReason::UnsupportedApp,
        };
    };
    if let Some(reason) = skip_reason(p, app) {
        return ConvertOutcome::Skipped {
            name: p.name.clone(),
            reason,
        };
    };
    ConvertOutcome::Imported(Provider {
        id: p.id.clone(),
        name: p.name.clone(),
        website_url: p.website_url.clone().unwrap_or_default(),
        category: p
            .category
            .as_deref()
            .map(map_category)
            .unwrap_or(ProviderCategory::Custom),
        app,
        icon: p.icon.clone().unwrap_or_default(),
        icon_color: p.icon_color.clone().unwrap_or_default(),
        sort_index: p.sort_index.map(|i| i.max(0) as u32).unwrap_or(0),
        notes: p.notes.clone().unwrap_or_default(),
        settings_config: serde_json::to_string(&p.settings_config).unwrap_or_else(|_| "{}".into()),
        meta: meta_to_raw(&p.meta),
        updated_at: now_iso.to_string(),
    })
}

/// 展开一个统一供应商为 claude / codex / gemini 各自的独立子 Provider。
/// `apps` 哪个为 true 就产出哪个应用的子 Provider；`target_app` 只产出该应用
/// 的子（单应用语境导入——claude 视图只搬 claude 部分）。id 前缀
/// `universal-<app>-`，category = `Aggregator`，name 沿用统一供应商名。展开前
/// 对统一供应商本身过一遍跳过判定（provider_type OAuth 类，或 base_url 命中
/// OAuth 端点）——newapi / custom 直通聚合网关，展开；命中则整组跳过。
pub fn expand_universal(
    u: &UniversalProvider,
    target_app: App,
    now_iso: &str,
) -> Vec<ConvertOutcome> {
    if is_oauth_provider_type(&u.provider_type)
        || u.base_url.contains("githubcopilot.com")
        || u.base_url.contains("chatgpt.com/backend-api/codex")
    {
        return vec![ConvertOutcome::Skipped {
            name: u.name.clone(),
            reason: SkipReason::NeedsOAuth,
        }];
    }
    let mut out = Vec::new();
    if App::Claude == target_app && u.apps.claude {
        out.push(ConvertOutcome::Imported(universal_child(
            App::Claude,
            &u.id,
            &u.name,
            universal_claude_settings(&u.base_url, &u.api_key),
            u,
            now_iso,
        )));
    }
    if App::Codex == target_app && u.apps.codex {
        out.push(ConvertOutcome::Imported(universal_child(
            App::Codex,
            &u.id,
            &u.name,
            universal_codex_settings(&u.base_url, &u.api_key, &u.name),
            u,
            now_iso,
        )));
    }
    if App::Gemini == target_app && u.apps.gemini {
        out.push(ConvertOutcome::Imported(universal_child(
            App::Gemini,
            &u.id,
            &u.name,
            universal_gemini_settings(&u.base_url, &u.api_key),
            u,
            now_iso,
        )));
    }
    out
}

// ── 辅助纯函数 ───────────────────────────────────────────────────────────────

/// CC-Switch `app_type` → cc one `App`。grokbuild / opencode 现已映射（取消搁置）。
fn map_app(app_type: &str) -> Option<App> {
    match app_type {
        "claude" => Some(App::Claude),
        "codex" => Some(App::Codex),
        "gemini" => Some(App::Gemini),
        "grokbuild" => Some(App::Grok),
        "opencode" => Some(App::OpenCode),
        _ => None,
    }
}

/// CC-Switch `category` 字符串 → cc one `ProviderCategory`。`third_party` / `omo` /
/// `omo-slim` 归 `Aggregator`（cc one 无 third_party 类目，语义最近）；缺失 / 未知 → `Custom`。
fn map_category(cat: &str) -> ProviderCategory {
    match cat {
        "official" => ProviderCategory::Official,
        "cn_official" => ProviderCategory::CnOfficial,
        "cloud_provider" => ProviderCategory::CloudProvider,
        "aggregator" | "third_party" | "omo" | "omo-slim" => ProviderCategory::Aggregator,
        "custom" => ProviderCategory::Custom,
        _ => ProviderCategory::Custom,
    }
}

/// 代理类 / OAuth 跳过判定。返回 `None` = 不跳过。
///
/// - OAuth：`meta.providerType` 命中 OAuth 类（[`is_oauth_provider_type`]），或
///   Claude 端点（`env.ANTHROPIC_BASE_URL`）命中 `githubcopilot.com` /
///   `chatgpt.com/backend-api/codex`。
/// - 代理：`meta.apiFormat` 命中 openai_chat / openai_responses / gemini_native
///   ——**仅对 claude/codex/gemini 判**：它们原生是 anthropic / openai-codex /
///   gemini-native，这些 apiFormat 表示中转；OpenCode / Grok 原生就是
///   openai-compatible / grok，apiFormat 对它们无意义，不据此误跳。
fn skip_reason(p: &CcSwitchProvider, app: App) -> Option<SkipReason> {
    let provider_type = string_field(&p.meta, "providerType");
    if is_oauth_provider_type(&provider_type) {
        return Some(SkipReason::NeedsOAuth);
    }
    if matches!(app, App::Claude | App::Codex | App::Gemini) {
        let api_format = string_field(&p.meta, "apiFormat");
        if matches!(
            api_format.as_str(),
            "openai_chat" | "openai_responses" | "gemini_native"
        ) {
            return Some(SkipReason::NeedsProxy);
        }
    }
    let base_url = settings_env_string(&p.settings_config, "ANTHROPIC_BASE_URL");
    if base_url.contains("githubcopilot.com") || base_url.contains("chatgpt.com/backend-api/codex")
    {
        return Some(SkipReason::NeedsOAuth);
    }
    None
}

/// OAuth 类 providerType（需 OAuth 账号，cc one 架构上不做代理）。`skip_reason`
/// 与 `expand_universal` 共用，避免 OAuth 集合两处分叉漂移。
fn is_oauth_provider_type(t: &str) -> bool {
    matches!(t, "github_copilot" | "codex_oauth" | "xai_oauth")
}

/// meta raw text：null / 非对象 → `"{}"`；否则原样序列化（保留代理层字段，写盘
/// 时自会剥非受控字段——导入层只搬运、不解释）。
fn meta_to_raw(meta: &Value) -> String {
    if meta.is_null() {
        return "{}".into();
    }
    serde_json::to_string(meta).unwrap_or_else(|_| "{}".into())
}

/// 从 JSON 对象取一个字符串字段（顶层），缺失 / 非字符串 → `""`。
fn string_field(obj: &Value, key: &str) -> String {
    obj.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// 从 settings_config（`{env:{...}}` 形状）的 env 取一个字符串值。
fn settings_env_string(settings: &Value, key: &str) -> String {
    settings
        .get("env")
        .and_then(|e| e.get(key))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// 统一供应商子 Provider 的 settings_config（各应用形状，模型用缺省）。
fn universal_claude_settings(base_url: &str, api_key: &str) -> Value {
    serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": base_url,
            "ANTHROPIC_AUTH_TOKEN": api_key,
            "ANTHROPIC_MODEL": "claude-sonnet-4-20250514",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5-20251001",
            "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-4-20250514",
            "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-opus-4-20250514"
        }
    })
}

fn universal_codex_settings(base_url: &str, api_key: &str, name: &str) -> Value {
    // 与 cc one codex 预设同形的 config TOML（model_provider = custom +
    // [model_providers.custom] 表），model 留空——用户在 cc one 内填或获取。用
    // toml_edit 构造以保证字符串正确转义（Rust Debug 格式不等于 TOML 转义）。
    let mut doc = DocumentMut::new();
    doc.insert("model_provider", toml_edit::value("custom"));
    doc.insert("model", toml_edit::value(""));
    let mut custom = Table::new();
    custom.insert("name", toml_edit::value(name));
    custom.insert("base_url", toml_edit::value(base_url));
    custom.insert("wire_api", toml_edit::value("responses"));
    custom.insert("requires_openai_auth", toml_edit::value(true));
    let mut providers = Table::new();
    providers.insert("custom", Item::Table(custom));
    doc.insert("model_providers", Item::Table(providers));
    serde_json::json!({ "auth": { "OPENAI_API_KEY": api_key }, "config": doc.to_string() })
}

fn universal_gemini_settings(base_url: &str, api_key: &str) -> Value {
    serde_json::json!({
        "env": {
            "GOOGLE_GEMINI_BASE_URL": base_url,
            "GEMINI_API_KEY": api_key,
            "GEMINI_MODEL": ""
        }
    })
}

/// 构造一个统一供应商展开的子 Provider。
fn universal_child(
    app: App,
    universal_id: &str,
    name: &str,
    settings: Value,
    u: &UniversalProvider,
    now_iso: &str,
) -> Provider {
    Provider {
        id: format!("universal-{}-{}", app.as_str(), universal_id),
        name: name.to_string(),
        website_url: u.website_url.clone().unwrap_or_default(),
        category: ProviderCategory::Aggregator,
        app,
        icon: u.icon.clone().unwrap_or_default(),
        icon_color: u.icon_color.clone().unwrap_or_default(),
        sort_index: 0,
        notes: String::new(),
        settings_config: serde_json::to_string(&settings).unwrap_or_else(|_| "{}".into()),
        meta: "{}".into(),
        updated_at: now_iso.to_string(),
    }
}

// ── 读盘 + 收集（命令层调用）─────────────────────────────────────────────────

/// 把已读入的 CC-Switch 供应商 + 统一供应商转换收集成（待写库的 cc one Provider，
/// 跳过明细）。纯收集：调 [`convert_ccswitch_provider`] / [`expand_universal`]，
/// 汇总 Imported / Skipped。
pub fn collect_ccswitch_imports(
    providers: &[CcSwitchProvider],
    universals: &[UniversalProvider],
    target_app: App,
    now_iso: &str,
) -> (Vec<Provider>, Vec<SkipDetail>) {
    let mut imported = Vec::new();
    let mut skipped = Vec::new();
    for p in providers {
        match convert_ccswitch_provider(p, now_iso) {
            ConvertOutcome::Imported(provider) => {
                // 单应用语境：只搬当前应用的部分（claude 视图不冒出 codex 供应商）。
                if provider.app == target_app {
                    imported.push(provider);
                }
            }
            ConvertOutcome::Skipped { name, reason } => skipped.push(SkipDetail { name, reason }),
        }
    }
    for u in universals {
        for outcome in expand_universal(u, target_app, now_iso) {
            match outcome {
                ConvertOutcome::Imported(provider) => imported.push(provider),
                ConvertOutcome::Skipped { name, reason } => {
                    skipped.push(SkipDetail { name, reason })
                }
            }
        }
    }
    (imported, skipped)
}

/// 从 CC-Switch SQLite 读 `providers` 表全部行为 [`CcSwitchProvider`]。
/// `settings_config` / `meta` 列是 JSON 文本，这里解析为 [`Value`]；解析失败按
/// null / `{}` 处理（容错——单条坏数据不让整个导入崩）。
pub fn read_providers_from_db(conn: &Connection) -> AppResult<Vec<CcSwitchProvider>> {
    let mut stmt = conn.prepare(
        "SELECT id, app_type, name, settings_config, website_url, category, \
         sort_index, notes, icon, icon_color, meta FROM providers",
    )?;
    let rows = stmt.query_map([], |row| {
        let settings_text: String = row.get(3)?;
        let meta_text: String = row.get::<_, Option<String>>(10)?.unwrap_or_default();
        let settings_config: Value = serde_json::from_str(&settings_text).unwrap_or(Value::Null);
        let meta: Value =
            serde_json::from_str(&meta_text).unwrap_or_else(|_| Value::Object(Default::default()));
        Ok(CcSwitchProvider {
            id: row.get(0)?,
            app_type: row.get(1)?,
            name: row.get(2)?,
            settings_config,
            website_url: row.get::<_, Option<String>>(4)?,
            category: row.get::<_, Option<String>>(5)?,
            sort_index: row.get::<_, Option<i64>>(6)?,
            notes: row.get::<_, Option<String>>(7)?,
            icon: row.get::<_, Option<String>>(8)?,
            icon_color: row.get::<_, Option<String>>(9)?,
            meta,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AppError::Config(format!("读取 CC-Switch providers 表失败: {e}")))
}

/// 从 CC-Switch SQLite 读 `settings` 表的 `universal_providers`（统一供应商 map）。
/// 键不存在 → 空列表（CC-Switch 可能没有统一供应商）。
pub fn read_universals_from_db(conn: &Connection) -> AppResult<Vec<UniversalProvider>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'universal_providers'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(text) = text else {
        return Ok(vec![]);
    };
    let map: std::collections::HashMap<String, UniversalProvider> = serde_json::from_str(&text)
        .map_err(|e| AppError::Config(format!("解析 universal_providers 失败: {e}")))?;
    Ok(map.into_values().collect())
}

/// 解析 CC-Switch 旧版 JSON（`config.json`，`MultiAppConfig` 结构）：`apps.<app_type>
/// .providers` 是 id → provider 的 map。id 取 map key、app_type 取外层 app key
/// （旧 JSON 的 provider 对象里通常没这俩字段，这里补上）。旧 JSON 不含统一供应商。
pub fn parse_legacy_json(text: &str) -> AppResult<(Vec<CcSwitchProvider>, Vec<UniversalProvider>)> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Config(format!("CC-Switch config.json 解析失败: {e}")))?;
    let mut providers = Vec::new();
    if let Some(apps) = v.get("apps").and_then(|a| a.as_object()) {
        for (app_type, app_cfg) in apps {
            if let Some(pm) = app_cfg.get("providers").and_then(|p| p.as_object()) {
                for (id, prov) in pm {
                    let mut prov = prov.clone();
                    if let Some(obj) = prov.as_object_mut() {
                        obj.insert("id".into(), Value::String(id.clone()));
                        obj.insert("appType".into(), Value::String(app_type.clone()));
                    }
                    if let Ok(p) = serde_json::from_value::<CcSwitchProvider>(prov) {
                        providers.push(p);
                    }
                }
            }
        }
    }
    Ok((providers, vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: &str = "2026-08-12T00:00:00Z";

    /// 一条典型 CC-Switch Claude 供应商（带 env/key/model/icon/category）。
    fn claude_provider() -> CcSwitchProvider {
        CcSwitchProvider {
            id: "abc".into(),
            name: "My Kimi".into(),
            app_type: "claude".into(),
            settings_config: json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.moonshot.cn/anthropic",
                    "ANTHROPIC_AUTH_TOKEN": "sk-xxx",
                    "ANTHROPIC_MODEL": "kimi-k2.7-code"
                }
            }),
            website_url: Some("https://platform.kimi.com".into()),
            category: Some("cn_official".into()),
            icon: Some("kimi".into()),
            icon_color: Some("#6366F1".into()),
            sort_index: Some(3),
            notes: None,
            meta: json!({}),
        }
    }

    #[test]
    fn 字段映射正确_claude() {
        let p = claude_provider();
        let ConvertOutcome::Imported(provider) = convert_ccswitch_provider(&p, NOW) else {
            panic!("应 Imported");
        };
        assert_eq!(provider.id, "abc");
        assert_eq!(provider.name, "My Kimi");
        assert_eq!(provider.website_url, "https://platform.kimi.com");
        assert_eq!(provider.category, ProviderCategory::CnOfficial);
        assert_eq!(provider.app, App::Claude);
        assert_eq!(provider.icon, "kimi");
        assert_eq!(provider.icon_color, "#6366F1");
        assert_eq!(provider.sort_index, 3);
        assert!(!provider.updated_at.is_empty());
        // settings_config 是等价 raw text（key 保留）。
        let sc: Value = serde_json::from_str(&provider.settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-xxx");
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "kimi-k2.7-code");
    }

    #[test]
    fn 五应用分别转换_都_imported() {
        // 取消搁置：grokbuild / opencode 现在也 Imported。
        let cases = [
            (
                "claude",
                json!({"env":{"ANTHROPIC_BASE_URL":"u","ANTHROPIC_AUTH_TOKEN":"k"}}),
                App::Claude,
            ),
            (
                "codex",
                json!({"auth":{"OPENAI_API_KEY":"k"},"config":"model = \"x\""}),
                App::Codex,
            ),
            ("gemini", json!({"env":{"GEMINI_API_KEY":"k"}}), App::Gemini),
            (
                "grokbuild",
                json!({"config":"[model.cc-one]\nmodel = \"grok-4.5\""}),
                App::Grok,
            ),
            (
                "opencode",
                json!({"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"u","apiKey":"k"}}),
                App::OpenCode,
            ),
        ];
        for (app_type, sc, expected_app) in cases {
            let p = CcSwitchProvider {
                id: format!("id-{app_type}"),
                name: app_type.into(),
                app_type: app_type.into(),
                settings_config: sc,
                website_url: None,
                category: None,
                icon: None,
                icon_color: None,
                sort_index: None,
                notes: None,
                meta: json!({}),
            };
            let outcome = convert_ccswitch_provider(&p, NOW);
            let ConvertOutcome::Imported(provider) = outcome else {
                panic!("{app_type} 应 Imported，得 {outcome:?}");
            };
            assert_eq!(provider.app, expected_app, "{app_type}");
        }
    }

    #[test]
    fn category_枚举映射() {
        fn convert(cat: &str) -> ProviderCategory {
            let p = CcSwitchProvider {
                id: "x".into(),
                name: "n".into(),
                app_type: "claude".into(),
                settings_config: json!({}),
                website_url: None,
                category: Some(cat.into()),
                icon: None,
                icon_color: None,
                sort_index: None,
                notes: None,
                meta: json!({}),
            };
            let ConvertOutcome::Imported(provider) = convert_ccswitch_provider(&p, NOW) else {
                panic!();
            };
            provider.category
        }
        assert_eq!(convert("official"), ProviderCategory::Official);
        assert_eq!(convert("cn_official"), ProviderCategory::CnOfficial);
        assert_eq!(convert("cloud_provider"), ProviderCategory::CloudProvider);
        assert_eq!(convert("aggregator"), ProviderCategory::Aggregator);
        assert_eq!(convert("third_party"), ProviderCategory::Aggregator);
        assert_eq!(convert("omo"), ProviderCategory::Aggregator);
        assert_eq!(convert("omo-slim"), ProviderCategory::Aggregator);
        assert_eq!(convert("custom"), ProviderCategory::Custom);
    }

    #[test]
    fn category_缺失或未知_归_custom() {
        let p = CcSwitchProvider {
            id: "x".into(),
            name: "n".into(),
            app_type: "claude".into(),
            settings_config: json!({}),
            website_url: None,
            category: None,
            icon: None,
            icon_color: None,
            sort_index: None,
            notes: None,
            meta: json!({}),
        };
        let ConvertOutcome::Imported(provider) = convert_ccswitch_provider(&p, NOW) else {
            panic!();
        };
        assert_eq!(provider.category, ProviderCategory::Custom);
    }

    #[test]
    fn 代理类跳过判定() {
        // NeedsOAuth：providerType
        let mut p = claude_provider();
        p.meta = json!({"providerType": "codex_oauth"});
        assert!(matches!(
            convert_ccswitch_provider(&p, NOW),
            ConvertOutcome::Skipped {
                reason: SkipReason::NeedsOAuth,
                ..
            }
        ));
        // NeedsProxy：apiFormat
        let mut p = claude_provider();
        p.meta = json!({"apiFormat": "openai_chat"});
        assert!(matches!(
            convert_ccswitch_provider(&p, NOW),
            ConvertOutcome::Skipped {
                reason: SkipReason::NeedsProxy,
                ..
            }
        ));
        // NeedsOAuth：base_url 命中 copilot 端点
        let mut p = claude_provider();
        p.settings_config = json!({"env":{"ANTHROPIC_BASE_URL":"https://api.githubcopilot.com"}});
        assert!(matches!(
            convert_ccswitch_provider(&p, NOW),
            ConvertOutcome::Skipped {
                reason: SkipReason::NeedsOAuth,
                ..
            }
        ));
        // 不跳过：apiFormat = anthropic 或缺失
        let p = claude_provider();
        assert!(matches!(
            convert_ccswitch_provider(&p, NOW),
            ConvertOutcome::Imported(_)
        ));
    }

    #[test]
    fn 代理类判定_对_codex_的_provider_type_也生效() {
        // codex_oauth 是 Codex 登录态——即使 app 支持也跳过。
        let p = CcSwitchProvider {
            id: "x".into(),
            name: "Codex Login".into(),
            app_type: "codex".into(),
            settings_config: json!({}),
            website_url: None,
            category: None,
            icon: None,
            icon_color: None,
            sort_index: None,
            notes: None,
            meta: json!({"providerType": "codex_oauth"}),
        };
        assert!(matches!(
            convert_ccswitch_provider(&p, NOW),
            ConvertOutcome::Skipped {
                reason: SkipReason::NeedsOAuth,
                ..
            }
        ));
    }

    #[test]
    fn 不支持的应用_跳过_unsupported_app() {
        for app_type in ["claude-desktop", "openclaw", "hermes", "unknown-app"] {
            let p = CcSwitchProvider {
                id: "x".into(),
                name: app_type.into(),
                app_type: app_type.into(),
                settings_config: json!({}),
                website_url: None,
                category: None,
                icon: None,
                icon_color: None,
                sort_index: None,
                notes: None,
                meta: json!({}),
            };
            assert!(
                matches!(
                    convert_ccswitch_provider(&p, NOW),
                    ConvertOutcome::Skipped {
                        reason: SkipReason::UnsupportedApp,
                        ..
                    }
                ),
                "{app_type} 应 UnsupportedApp"
            );
        }
    }

    #[test]
    fn 凭据保留_settings_config_含原_key() {
        let p = claude_provider();
        let ConvertOutcome::Imported(provider) = convert_ccswitch_provider(&p, NOW) else {
            panic!();
        };
        let sc: Value = serde_json::from_str(&provider.settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-xxx");
    }

    #[test]
    fn meta_往返_保留代理层字段() {
        let mut p = claude_provider();
        p.meta = json!({"apiFormat": "anthropic", "customUserAgent": "x", "costMultiplier": 1.5});
        let ConvertOutcome::Imported(provider) = convert_ccswitch_provider(&p, NOW) else {
            panic!();
        };
        let meta: Value = serde_json::from_str(&provider.meta).unwrap();
        assert_eq!(meta["customUserAgent"], "x");
        assert_eq!(meta["costMultiplier"], 1.5);
    }

    #[test]
    fn 统一供应商展开_按_apps_产出子_provider() {
        let u = UniversalProvider {
            id: "gw1".into(),
            name: "My Gateway".into(),
            provider_type: "newapi".into(),
            apps: UniversalApps {
                claude: true,
                codex: true,
                gemini: false,
            },
            base_url: "https://gw.example.com/v1".into(),
            api_key: "sk-gw".into(),
            website_url: None,
            icon: None,
            icon_color: None,
        };
        // 单应用语境：target_app 是 claude → 只产 claude 子（codex 的 apps 虽开
        // 但不产）。
        let outcomes = expand_universal(&u, App::Claude, NOW);
        assert_eq!(
            outcomes.len(),
            1,
            "只产 target_app（claude），codex/gemini 不产"
        );
        let ConvertOutcome::Imported(claude) = &outcomes[0] else {
            panic!("应 Imported");
        };
        assert_eq!(claude.app, App::Claude);
        assert!(claude.id.starts_with("universal-"));
        // id 前缀 + category。
        for o in &outcomes {
            let ConvertOutcome::Imported(p) = o else {
                panic!();
            };
            assert!(p.id.starts_with("universal-"));
            assert_eq!(p.category, ProviderCategory::Aggregator);
            assert_eq!(p.name, "My Gateway");
        }
        // claude 子的 settings_config 形状。
        let claude = outcomes
            .iter()
            .find_map(|o| match o {
                ConvertOutcome::Imported(p) if p.app == App::Claude => Some(p),
                _ => None,
            })
            .unwrap();
        let sc: Value = serde_json::from_str(&claude.settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_BASE_URL"], "https://gw.example.com/v1");
        assert_eq!(sc["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-gw");
        assert_eq!(claude.id, "universal-claude-gw1");
    }

    #[test]
    fn 统一供应商_所有_app_关闭_产出空() {
        let u = UniversalProvider {
            id: "gw2".into(),
            name: "Empty".into(),
            provider_type: "newapi".into(),
            apps: UniversalApps::default(),
            base_url: "".into(),
            api_key: "".into(),
            website_url: None,
            icon: None,
            icon_color: None,
        };
        assert!(expand_universal(&u, App::Claude, NOW).is_empty());
    }

    #[test]
    fn opencode_不判_apiformat_openai_chat_仍导入() {
        // OpenCode 原生是 openai-compatible，apiFormat=openai_chat 对它无意义——
        // 不应被 NeedsProxy 误跳（apiFormat 代理判定仅对 claude/codex/gemini）。
        let p = CcSwitchProvider {
            id: "x".into(),
            name: "OC".into(),
            app_type: "opencode".into(),
            settings_config: json!({
                "npm": "@ai-sdk/openai-compatible",
                "options": { "baseURL": "https://api.x", "apiKey": "k" }
            }),
            website_url: None,
            category: None,
            icon: None,
            icon_color: None,
            sort_index: None,
            notes: None,
            meta: json!({ "apiFormat": "openai_chat" }),
        };
        assert!(matches!(
            convert_ccswitch_provider(&p, NOW),
            ConvertOutcome::Imported(_)
        ));
    }

    #[test]
    fn 统一供应商_base_url_命中_oauth_端点_整组跳过() {
        // provider_type 非 OAuth，但 base_url 命中 copilot 端点——展开前判定命中，
        // 整组跳过（不产出任何子 Provider）。
        let u = UniversalProvider {
            id: "gw3".into(),
            name: "Copilot GW".into(),
            provider_type: "newapi".into(),
            apps: UniversalApps {
                claude: true,
                codex: true,
                gemini: true,
            },
            base_url: "https://api.githubcopilot.com".into(),
            api_key: "k".into(),
            website_url: None,
            icon: None,
            icon_color: None,
        };
        let outcomes = expand_universal(&u, App::Claude, NOW);
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(
            outcomes[0],
            ConvertOutcome::Skipped {
                reason: SkipReason::NeedsOAuth,
                ..
            }
        ));
    }
}
