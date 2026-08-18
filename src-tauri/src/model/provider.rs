//! Provider (供应商) entity types.
//!
//! One provider is a vendor CC One can switch one of its apps (Claude Code /
//! Codex CLI / Gemini CLI) to: a `settings.json` snapshot (`settingsConfig`)
//! plus app-side extras (`meta`). Every provider belongs to exactly one
//! [`App`]; the merge/dedup key across sync and export/import is `(app, id)`,
//! so the same vendor appears as separate entries in each app's pool (their
//! live config formats differ). The snapshot is the single authority — every
//! form field, preset and snippet reads/writes it — and API keys live in
//! app-specific locations inside it (`env` / `auth` / `options`, see
//! [`crate::provider::keys`], the single source of truth). Both
//! `settingsConfig` and `meta` cross the boundary as raw JSON *text*: the
//! store persists them as TEXT as-is, and the future CodeMirror editor edits
//! that text directly, so nothing here parses or prettifies it (that is the
//! frontend `derive.ts`'s job).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::AppResult;

/// The app (应用) a provider pool belongs to. Each app owns an independent
/// provider pool, per-app active state and per-app common-config snippet.
/// Serialized snake_case ("claude" / "codex" / "gemini" / "grok" / "opencode")
/// — the same spelling crosses as JSON, the sync file and the DB.
///
/// 两种 mode（[`App::is_additive_mode`]）：claude/codex/gemini/grok 是**单激活**
/// （一个 app 一个活跃 provider，切换=替换，写盘整文件受控合并）；opencode 是
/// **附加**（多供应商共存于 opencode.json 的 `provider.<id>` map，无唯一活跃，
/// 增删单条）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum App {
    /// Claude Code — the original pool; existing data all belongs here.
    #[default]
    Claude,
    /// Codex CLI.
    Codex,
    /// Gemini CLI.
    Gemini,
    /// Grok CLI.
    Grok,
    /// OpenCode — 附加模式（additive）：多供应商共存于 opencode.json 的
    /// `provider.<id>` map，无唯一活跃。写盘走单键 read-modify-write
    /// （`live_opencode`），不进 `write_live`。
    #[serde(rename = "opencode")]
    OpenCode,
}

impl App {
    /// The stored spelling (DB column / config.json key; identical to the
    /// serde snake_case JSON spelling).
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            App::Claude => "claude",
            App::Codex => "codex",
            App::Gemini => "gemini",
            App::Grok => "grok",
            App::OpenCode => "opencode",
        }
    }

    /// Parse the stored spelling; anything unrecognised falls back to
    /// [`App::Claude`] so an unknown value (a typo, a future app) never fails
    /// the whole list read — the sync-file version gate is the stricter guard
    /// for files this binary cannot attribute at all.
    pub(crate) fn from_db_str(s: &str) -> App {
        match s {
            "claude" => App::Claude,
            "codex" => App::Codex,
            "gemini" => App::Gemini,
            "grok" => App::Grok,
            "opencode" => App::OpenCode,
            _ => App::Claude,
        }
    }

    /// 附加模式（additive）应用返回 true：多供应商共存于配置文件，无唯一
    /// 活跃，写盘走单键 read-modify-write。单激活应用返回 false。所有「单激活
    /// vs 附加」分派走这里（单一事实来源），禁止散落 `if app == App::OpenCode`。
    pub(crate) fn is_additive_mode(self) -> bool {
        matches!(self, App::OpenCode)
    }
}

/// Provider category. `Custom` is the value for user-created providers; the
/// rest label and theme the built-in presets in the list view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCategory {
    Official,
    CnOfficial,
    Aggregator,
    CloudProvider,
    Custom,
}

impl ProviderCategory {
    /// The SQLite-stored spelling (also the JSON spelling via `rename_all`).
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ProviderCategory::Official => "official",
            ProviderCategory::CnOfficial => "cn_official",
            ProviderCategory::Aggregator => "aggregator",
            ProviderCategory::CloudProvider => "cloud_provider",
            ProviderCategory::Custom => "custom",
        }
    }

    /// Parse the SQLite-stored spelling; anything unrecognised falls back to
    /// `Custom` so an unknown value (a typo, a future category) never fails
    /// the whole list read.
    pub(crate) fn from_db_str(s: &str) -> ProviderCategory {
        match s {
            "official" => ProviderCategory::Official,
            "cn_official" => ProviderCategory::CnOfficial,
            "aggregator" => ProviderCategory::Aggregator,
            "cloud_provider" => ProviderCategory::CloudProvider,
            _ => ProviderCategory::Custom,
        }
    }
}

/// A provider (供应商): `settingsConfig` is the owning app's live-file
/// snapshot (raw JSON text) — Claude 是 `settings.json` 快照，Codex 是
/// `{"auth", "config"}` 快照（auth = auth.json 内容、config = config.toml
/// TOML 文本）；`meta` carries app-side info the live file never sees.
/// `sortIndex` is the user-ordered display rank *within the provider's app
/// pool*. Missing `app` in a JSON document (old sync files, old exports)
/// deserializes as `Claude` — the pre-app-dimension data all belongs there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub website_url: String,
    pub category: ProviderCategory,
    /// 归属应用；合并/去重键是 (app, id)。缺省读为 claude（旧数据/旧文件）。
    #[serde(default)]
    pub app: App,
    pub icon: String,
    pub icon_color: String,
    pub sort_index: u32,
    pub notes: String,
    /// 应用 live 文件快照，raw JSON text：claude = `settings.json` 内容；
    /// codex = `{"auth": ..., "config": "TOML"}` 对象（auth 镜像 auth.json）。
    pub settings_config: String,
    /// App-side extras, raw JSON text. Never written to the live file.
    pub meta: String,
    pub updated_at: String,
}

/// 通用配置片段（全局一条，跨供应商共享）：手写 settings.json 片段 +
/// 勾选启用，切换写盘时合并进受控字段。存本机 config.json（不同步）；
/// `content` 是片段 JSON 原文，编辑器直接编辑。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommonConfigSnippet {
    /// 勾选「应用通用配置」启用；写盘时片段合并进受控字段。
    pub enabled: bool,
    /// 片段 JSON 原文（原始文本；空串合法 = 无操作片段）。
    pub content: String,
}

/// A short random id for a user-created provider (8 lowercase hex chars — the
/// same generator as the sessions' local group ids; both are device-local id
/// spaces that never leave the machine, so a prefix is unnecessary).
pub(crate) fn generate_provider_id() -> String {
    crate::sessions::generate_local_group_id()
}

impl Provider {
    /// The sync-safe projection: the key locations defined in
    /// [`crate::provider::keys`] — `settingsConfig`'s `env` / `auth` objects,
    /// opencode's `options.apiKey` / `options.headers` auth-header whitelist,
    /// and `meta.templateValues` (the frontend's record of filled `${VAR}`
    /// template variables, which is how the Bedrock presets carry AK/SK) —
    /// are stripped from this row. Thin shell over
    /// [`crate::provider::keys::strip_settings_config`] /
    /// [`crate::provider::keys::strip_meta`]: a surface that carries no
    /// secret is kept verbatim (byte-stable across pushes); one that did
    /// carry a secret is re-serialized deterministically (serde_json's
    /// default `Value` map sorts keys). Returns `Err` when the config is not
    /// valid JSON / not an object / has a non-object `env` or `auth`, or the
    /// meta cannot be parsed — a provider whose secrets cannot be proven
    /// absent must not be published (the sync writer skips it).
    pub fn redacted(&self) -> AppResult<Provider> {
        let settings_config =
            crate::provider::keys::strip_settings_config(&self.settings_config, "provider")?;
        let meta = crate::provider::keys::strip_meta(&self.meta, "provider")?;
        if settings_config == self.settings_config && meta == self.meta {
            return Ok(self.clone());
        }
        let mut p = self.clone();
        p.settings_config = settings_config;
        p.meta = meta;
        Ok(p)
    }

    /// True iff two rows carry identical syncable structure: every field that
    /// syncs — including the key-stripped `settingsConfig` and key-stripped
    /// `meta` — except `sort_index` (never set through save) and `updated_at`
    /// (the computed freshness). Secret keys don't count (stripped before
    /// compare, in both surfaces), so a key-only edit compares equal. A
    /// provider whose config cannot be parsed never compares equal — treat
    /// that as a structural change, never assume.
    pub fn structure_equals(&self, other: &Provider) -> bool {
        if self.id != other.id
            || self.name != other.name
            || self.website_url != other.website_url
            || self.category != other.category
            || self.app != other.app
            || self.icon != other.icon
            || self.icon_color != other.icon_color
            || self.notes != other.notes
        {
            return false;
        }
        // The fields above are already compared; the key-stripped config and
        // meta are all that remains. Compare the redacted values — parsed as
        // JSON so redaction's re-serialization (pretty-printed when a key was
        // stripped on one side only) can't make an equal pair look different.
        // The clones still carry each row's `updated_at`, which must not
        // count.
        match (self.redacted(), other.redacted()) {
            (Ok(a), Ok(b)) => {
                json_text_eq(&a.settings_config, &b.settings_config)
                    && json_text_eq(&a.meta, &b.meta)
            }
            _ => false,
        }
    }
}

/// Compare two raw-JSON-text fields for structural equality: parsed values
/// when both sides parse (robust to redaction re-serialization), verbatim
/// otherwise — a blank or unparseable field only equals itself.
fn json_text_eq(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::keys::SECRET_ENV_KEYS;

    #[test]
    fn category_db_str_roundtrips() {
        for c in [
            ProviderCategory::Official,
            ProviderCategory::CnOfficial,
            ProviderCategory::Aggregator,
            ProviderCategory::CloudProvider,
            ProviderCategory::Custom,
        ] {
            assert_eq!(ProviderCategory::from_db_str(c.as_str()), c);
        }
    }

    #[test]
    fn category_unknown_db_str_falls_back_to_custom() {
        assert_eq!(
            ProviderCategory::from_db_str("bogus"),
            ProviderCategory::Custom
        );
    }

    #[test]
    fn app_db_str_roundtrips_and_defaults_to_claude() {
        assert_eq!(App::from_db_str(App::Claude.as_str()), App::Claude);
        assert_eq!(App::from_db_str(App::Codex.as_str()), App::Codex);
        assert_eq!(App::from_db_str(App::Gemini.as_str()), App::Gemini);
        assert_eq!(App::from_db_str(App::Grok.as_str()), App::Grok);
        assert_eq!(App::default(), App::Claude);
        assert_eq!(App::Claude.as_str(), "claude");
        assert_eq!(App::Codex.as_str(), "codex");
        assert_eq!(App::Gemini.as_str(), "gemini");
        assert_eq!(App::Grok.as_str(), "grok");
    }

    #[test]
    fn app_unknown_db_str_falls_back_to_claude() {
        assert_eq!(App::from_db_str("bogus"), App::Claude);
        assert_eq!(App::from_db_str(""), App::Claude);
    }

    /// 防回归：只有 OpenCode 是附加模式，其余四个单激活应用都不是。所有
    /// 「单激活 vs 附加」分派都走 is_additive_mode——加新应用时这个测试守住
    /// 它必须显式归类（漏归类会被红灯抓住）。
    #[test]
    fn is_additive_mode_only_true_for_opencode() {
        assert!(App::OpenCode.is_additive_mode());
        assert!(!App::Claude.is_additive_mode());
        assert!(!App::Codex.is_additive_mode());
        assert!(!App::Gemini.is_additive_mode());
        assert!(!App::Grok.is_additive_mode());
    }

    /// A provider JSON document without an `app` field (pre-app-dimension
    /// sync lines / exports) reads as Claude — old data all belongs there.
    #[test]
    fn provider_without_app_field_deserializes_as_claude() {
        let json = r#"{"id":"p1","name":"Kimi","websiteUrl":"https://x.dev","category":"custom","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{}","meta":"{}","updatedAt":"2026-08-01T00:00:00Z"}"#;
        let p: Provider = serde_json::from_str(json).unwrap();
        assert_eq!(p.app, App::Claude);
    }

    #[test]
    fn provider_serializes_camel_case() {
        let p = Provider {
            id: "p1".into(),
            name: "Kimi".into(),
            website_url: "https://platform.kimi.com".into(),
            category: ProviderCategory::CnOfficial,
            app: App::Claude,
            icon: "kimi".into(),
            icon_color: "#6366F1".into(),
            sort_index: 0,
            notes: String::new(),
            settings_config: r#"{"env":{}}"#.into(),
            meta: r#"{}"#.into(),
            updated_at: "2026-08-07T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"websiteUrl\""));
        assert!(json.contains("\"sortIndex\""));
        assert!(json.contains("\"settingsConfig\""));
        assert!(json.contains("\"cn_official\""));
        assert!(json.contains("\"app\":\"claude\""));
        // The raw JSON text fields stay raw — the value is escaped (inner
        // quotes → `\"`) but never re-parsed or prettified.
        assert!(json.contains(r#""settingsConfig":"{\"env\":{}}""#));
    }

    #[test]
    fn provider_id_is_eight_hex_chars() {
        let id = generate_provider_id();
        assert_eq!(id.len(), 8);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    /// A provider whose env carries all four secret keys plus a region, a
    /// base URL and a model.
    fn keyed_provider() -> Provider {
        Provider {
            id: "p1".into(),
            name: "Bedrock".into(),
            website_url: "https://bedrock.aws".into(),
            category: ProviderCategory::CloudProvider,
            app: App::Claude,
            icon: "bedrock".into(),
            icon_color: "#ff0".into(),
            sort_index: 2,
            notes: "n".into(),
            settings_config: r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.bedrock","ANTHROPIC_AUTH_TOKEN":"sk-token","ANTHROPIC_API_KEY":"sk-key","AWS_ACCESS_KEY_ID":"AKIA123","AWS_SECRET_ACCESS_KEY":"top-secret","AWS_REGION":"us-east-1","ANTHROPIC_MODEL":"claude-sonnet"},"includeCoAuthoredBy":false}"#.into(),
            meta: r#"{"auth_field":"aws"}"#.into(),
            updated_at: "2026-08-01T00:00:00.000Z".into(),
        }
    }

    #[test]
    fn redacted_strips_secret_keys_and_keeps_region_and_structure() {
        let p = keyed_provider();
        let r = p.redacted().unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.settings_config).unwrap();
        let env = &v["env"];
        for key in SECRET_ENV_KEYS {
            assert!(env.get(*key).is_none(), "{key} must be stripped");
        }
        // Non-secret env entries survive; AWS_REGION is a region/template
        // placeholder, not a credential.
        assert_eq!(env["AWS_REGION"], serde_json::json!("us-east-1"));
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            serde_json::json!("https://api.bedrock")
        );
        assert_eq!(env["ANTHROPIC_MODEL"], serde_json::json!("claude-sonnet"));
        assert_eq!(v["includeCoAuthoredBy"], serde_json::json!(false));
        // The rest of the row is untouched.
        assert_eq!(r.id, p.id);
        assert_eq!(r.name, p.name);
        assert_eq!(r.sort_index, p.sort_index);
        assert_eq!(r.updated_at, p.updated_at);
        assert_eq!(r.meta, p.meta);
        // Redaction is idempotent and byte-stable.
        assert_eq!(
            r.settings_config,
            r.redacted().unwrap().settings_config,
            "redacting twice must not churn the bytes"
        );
        // The secret key names never appear anywhere in the projection.
        for key in SECRET_ENV_KEYS {
            assert!(!r.settings_config.contains(key));
        }
    }

    #[test]
    fn redacted_strips_secret_template_values_from_meta() {
        // Bedrock presets route AK/SK through `${VAR}` template variables,
        // whose filled values are recorded in meta.templateValues — those are
        // credentials and must be stripped from the sync projection too.
        let mut p = keyed_provider();
        p.meta = r#"{"templateValues":{"AWS_REGION":"us-east-1","AWS_ACCESS_KEY_ID":"AKIA123","AWS_SECRET_ACCESS_KEY":"top-secret","ANTHROPIC_AUTH_TOKEN":"sk-token"}}"#.into();
        let r = p.redacted().unwrap();
        let meta: serde_json::Value = serde_json::from_str(&r.meta).unwrap();
        let values = &meta["templateValues"];
        assert!(values.get("AWS_ACCESS_KEY_ID").is_none(), "AK stripped");
        assert!(values.get("AWS_SECRET_ACCESS_KEY").is_none(), "SK stripped");
        assert!(values.get("ANTHROPIC_AUTH_TOKEN").is_none());
        // Non-secret template values survive.
        assert_eq!(values["AWS_REGION"], serde_json::json!("us-east-1"));
        // The stripped names never appear anywhere in the projection.
        for key in SECRET_ENV_KEYS {
            assert!(!r.meta.contains(key), "{key} must not appear in meta");
        }
        assert!(!r.settings_config.contains("ANTHROPIC_AUTH_TOKEN"));
    }

    #[test]
    fn redacted_meta_template_values_alone_triggers_rewrite() {
        // A provider whose env has no secrets but whose meta carries AK/SK:
        // the meta is rewritten while the config stays byte-identical.
        let mut p = keyed_provider();
        p.settings_config = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.bedrock"}}"#.into();
        p.meta = r#"{"templateValues":{"AWS_SECRET_ACCESS_KEY":"s3cret"}}"#.into();
        let r = p.redacted().unwrap();
        assert_eq!(
            r.settings_config, p.settings_config,
            "config without secrets stays verbatim"
        );
        let meta: serde_json::Value = serde_json::from_str(&r.meta).unwrap();
        assert!(meta.get("templateValues").is_none());
    }

    #[test]
    fn redacted_rejects_unparseable_meta() {
        let mut p = keyed_provider();
        p.meta = "{oops".into();
        assert!(
            p.redacted().is_err(),
            "unparseable meta cannot prove secrets absent"
        );
        // But an empty meta (no template values at all) is fine.
        let mut q = keyed_provider();
        q.meta = "  ".into();
        assert!(q.redacted().is_ok());
    }

    #[test]
    fn redacted_passes_through_blank_config_and_config_without_secrets() {
        let mut blank = keyed_provider();
        blank.settings_config = "  ".into();
        assert_eq!(blank.redacted().unwrap().settings_config, "  ");

        let mut plain = keyed_provider();
        plain.settings_config =
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://x.dev"},"includeCoAuthoredBy":false}"#.into();
        let r = plain.redacted().unwrap();
        // Nothing was stripped ⇒ the authored text is kept verbatim.
        assert_eq!(r.settings_config, plain.settings_config);

        // No env block at all ⇒ nothing to strip.
        let mut no_env = keyed_provider();
        no_env.settings_config = r#"{"includeCoAuthoredBy":false}"#.into();
        assert_eq!(
            no_env.redacted().unwrap().settings_config,
            no_env.settings_config
        );
    }

    #[test]
    fn redacted_rejects_unparseable_or_non_object_config() {
        let mut bad = keyed_provider();
        bad.settings_config = "{oops".into();
        assert!(bad.redacted().is_err(), "invalid JSON must error");
        bad.settings_config = r#"[1,2]"#.into();
        assert!(bad.redacted().is_err(), "non-object must error");
        bad.settings_config = r#"{"env":"nope"}"#.into();
        assert!(bad.redacted().is_err(), "non-object env must error");
    }

    /// Codex 供应商的 `auth` 对象镜像 auth.json：`OPENAI_API_KEY` 必须随
    /// `SECRET_ENV_KEYS` 一起剥掉，密钥名绝不出现在同步投影里。
    #[test]
    fn redacted_strips_codex_auth_key() {
        let mut p = keyed_provider();
        p.settings_config =
            r#"{"auth":{"OPENAI_API_KEY":"sk-codex-123"},"config":"model = \"gpt-5.6\""}"#.into();
        let r = p.redacted().unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.settings_config).unwrap();
        assert!(
            v["auth"].get("OPENAI_API_KEY").is_none(),
            "codex key stripped"
        );
        assert!(
            !r.settings_config.contains("OPENAI_API_KEY"),
            "key name must not appear in the projection"
        );
        assert!(!r.settings_config.contains("sk-codex-123"));
        // 非密钥字段（config TOML 文本）原样保留。
        assert_eq!(v["config"], serde_json::json!("model = \"gpt-5.6\""));
        // 幂等且字节稳定。
        assert_eq!(r.settings_config, r.redacted().unwrap().settings_config);
    }

    #[test]
    fn redacted_rejects_non_object_auth() {
        let mut p = keyed_provider();
        p.settings_config = r#"{"auth":"sk-plain-string","config":""}"#.into();
        assert!(
            p.redacted().is_err(),
            "非对象 auth 无法证明密钥缺失，必须拒绝发布"
        );
    }

    /// OpenCode 供应商的密钥在 `options.apiKey` + `options.headers` 认证头白名单：
    /// 两者必须随同步投影剥掉，元数据头（Helicone-*）保留。
    #[test]
    fn redacted_strips_opencode_options_apikey_and_auth_headers() {
        let mut p = keyed_provider();
        p.app = App::OpenCode;
        p.settings_config = r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.deepseek.com","apiKey":"sk-opencode","headers":{"Authorization":"Bearer tok","Helicone-Auth":"meta","x-api-key":"k2"}}}"#.into();
        let r = p.redacted().unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.settings_config).unwrap();
        assert!(
            v["options"].get("apiKey").is_none(),
            "options.apiKey 必须剥"
        );
        assert!(
            v["options"]["headers"].get("Authorization").is_none(),
            "Authorization 必须剥"
        );
        assert!(
            v["options"]["headers"].get("x-api-key").is_none(),
            "x-api-key 必须剥"
        );
        // 元数据头（非凭据）保留。
        assert_eq!(v["options"]["headers"]["Helicone-Auth"], "meta");
        // 非密钥字段保留。
        assert_eq!(v["options"]["baseURL"], "https://api.deepseek.com");
        assert_eq!(v["npm"], "@ai-sdk/openai-compatible");
        // 密钥值与键名都不出现在投影里。
        assert!(!r.settings_config.contains("sk-opencode"));
        assert!(!r.settings_config.contains("Bearer tok"));
        // 幂等且字节稳定。
        assert_eq!(r.settings_config, r.redacted().unwrap().settings_config);
    }

    #[test]
    fn redacted_strips_opencode_headers_case_insensitively() {
        // HTTP header 大小写不敏感：用户写小写 authorization / 混合大小写也剥。
        let mut p = keyed_provider();
        p.app = App::OpenCode;
        p.settings_config = r#"{"options":{"apiKey":"k","headers":{"authorization":"tok","PROXY-AUTHORIZATION":"p"}}}"#.into();
        let r = p.redacted().unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.settings_config).unwrap();
        assert!(v["options"]["headers"].get("authorization").is_none());
        assert!(v["options"]["headers"].get("PROXY-AUTHORIZATION").is_none());
        assert!(!r.settings_config.contains("tok"));
    }

    #[test]
    fn redacted_skips_non_object_options_without_error() {
        // options 非对象（坏数据）——不像 env/auth 报错，跳过（不阻止发布）。
        let mut p = keyed_provider();
        p.settings_config =
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://x"},"options":"garbage"}"#.into();
        let r = p.redacted().unwrap();
        let v: serde_json::Value = serde_json::from_str(&r.settings_config).unwrap();
        assert_eq!(v["options"], "garbage", "非对象 options 原样保留，不报错");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://x");
    }

    /// 旧同步文件 / 导出文档无 app 字段 → 读为 claude（向后兼容的永久规则：
    /// app 维度落地前的数据都在 claude 池）；带 app 字段的新文档按值解析。
    #[test]
    fn provider_app_missing_defaults_to_claude_and_value_roundtrips() {
        let old: Provider = serde_json::from_str(
            r#"{"id":"p1","name":"Kimi","websiteUrl":"https://x.dev","category":"custom","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{}","meta":"{}","updatedAt":"2026-08-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(old.app, App::Claude);

        let codex: Provider = serde_json::from_str(
            r#"{"app":"codex","id":"p2","name":"Kimi","websiteUrl":"https://x.dev","category":"custom","icon":"","iconColor":"","sortIndex":0,"notes":"","settingsConfig":"{}","meta":"{}","updatedAt":"2026-08-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(codex.app, App::Codex);
        let back: Provider = serde_json::from_str(&serde_json::to_string(&codex).unwrap()).unwrap();
        assert_eq!(back.app, App::Codex);
    }

    #[test]
    fn structure_equals_ignores_keys_and_freshness_but_not_other_fields() {
        let base = keyed_provider();

        // A key-only edit compares equal (structure unchanged).
        let mut keyed = base.clone();
        keyed.settings_config = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.bedrock","ANTHROPIC_AUTH_TOKEN":"sk-NEW-token","AWS_REGION":"us-east-1","ANTHROPIC_MODEL":"claude-sonnet"},"includeCoAuthoredBy":false}"#.into();
        keyed.updated_at = "2026-08-02T00:00:00.000Z".into();
        assert!(base.structure_equals(&keyed), "key edit is not structural");

        // A template-value key edit in meta is not structural either (the
        // original auth_field entry is preserved so only the key differs).
        let mut keyed_meta = base.clone();
        keyed_meta.meta =
            r#"{"auth_field":"aws","templateValues":{"AWS_SECRET_ACCESS_KEY":"s3cret"}}"#.into();
        assert!(
            base.structure_equals(&keyed_meta),
            "meta template-value key edit is not structural"
        );

        // A structural edit (name, endpoint, model…) differs.
        let mut renamed = base.clone();
        renamed.name = "Bedrock Pro".into();
        assert!(!base.structure_equals(&renamed));
        let mut moved = base.clone();
        moved.settings_config = r#"{"env":{"ANTHROPIC_BASE_URL":"https://other.dev","ANTHROPIC_AUTH_TOKEN":"sk-token"}}"#.into();
        assert!(!base.structure_equals(&moved));

        // An unparseable config never compares equal.
        let mut broken = base.clone();
        broken.settings_config = "{oops".into();
        assert!(!base.structure_equals(&broken));
    }
}
