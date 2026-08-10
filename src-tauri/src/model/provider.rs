//! Provider (供应商) entity types.
//!
//! One provider is a vendor CC One can switch one of its apps (Claude Code /
//! Codex CLI / Gemini CLI) to: a `settings.json` snapshot (`settingsConfig`)
//! plus app-side extras (`meta`). Every provider belongs to exactly one
//! [`App`]; the merge/dedup key across sync and export/import is `(app, id)`,
//! so the same vendor appears as separate entries in each app's pool (their
//! live config formats differ). The snapshot is the single authority — every
//! form field, preset and snippet reads/writes it — and API keys live inside
//! its `env` block (`ANTHROPIC_AUTH_TOKEN` / `ANTHROPIC_API_KEY`). Both
//! `settingsConfig` and `meta` cross the boundary as raw JSON *text*: the
//! store persists them as TEXT as-is, and the future CodeMirror editor edits
//! that text directly, so nothing here parses or prettifies it (that is the
//! frontend `derive.ts`'s job).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::{AppError, AppResult};

/// The app (应用) a provider pool belongs to. Each app owns an independent
/// provider pool, per-app active state and per-app common-config snippet.
/// Serialized snake_case ("claude" / "codex" / "gemini") — the same spelling
/// crosses as JSON, the sync file and the DB.
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
}

impl App {
    /// The stored spelling (DB column / config.json key; identical to the
    /// serde snake_case JSON spelling).
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            App::Claude => "claude",
            App::Codex => "codex",
            App::Gemini => "gemini",
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
            _ => App::Claude,
        }
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

/// Secret env-var keys stripped from `settingsConfig` before it leaves this
/// device (the synced `providers.json`): API keys live in the `env` block
/// (claude `ANTHROPIC_*` / gemini `GEMINI_API_KEY`) or the `auth` object
/// (codex `OPENAI_API_KEY`) and must never enter the repo. `AWS_REGION` is
/// deliberately NOT here — it is a non-secret region code (or a `${VAR}`
/// template-variable placeholder), not a credential. This list is the single
/// source of truth: `Provider::redacted`, the sync merge, and the export path
/// (`provider::export_import`) all route through it.
pub const SECRET_ENV_KEYS: &[&str] = &[
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_ACCESS_KEY_ID",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
];

impl Provider {
    /// The sync-safe projection: [`SECRET_ENV_KEYS`] removed from three places —
    /// the `settingsConfig` `env` object, the `settingsConfig` `auth` object
    /// (codex providers carry `OPENAI_API_KEY` there — the auth.json mirror),
    /// and the `meta.templateValues` object. The `env` block is where claude
    /// API keys normally live; `meta.templateValues` is the frontend's record
    /// of filled `${VAR}` template variables, and the Bedrock presets route
    /// AK/SK through those, so a redaction that stops at `env` would still
    /// publish credentials. Blank config passes through unchanged (nothing to
    /// strip); a config or meta that carries a secret key is re-serialized
    /// deterministically (serde_json's default `Value` map sorts keys), so the
    /// written file is byte-stable across pushes. Returns `Err` when the config
    /// is not valid JSON / not an object / has a non-object `env` or `auth`, or
    /// the meta cannot be parsed to an object — a provider whose secrets cannot
    /// be proven absent must not be published (the sync writer skips it).
    pub fn redacted(&self) -> AppResult<Provider> {
        let trimmed = self.settings_config.trim();
        if trimmed.is_empty() {
            return Ok(self.clone());
        }
        let mut v: serde_json::Value = serde_json::from_str(trimmed).map_err(|e| {
            AppError::Config(format!("provider settingsConfig is not valid JSON: {e}"))
        })?;
        let obj = v.as_object_mut().ok_or_else(|| {
            AppError::Config("provider settingsConfig is not a JSON object".into())
        })?;
        let mut stripped = false;
        if let Some(env) = obj.get_mut("env") {
            let env = env.as_object_mut().ok_or_else(|| {
                AppError::Config("provider settingsConfig env is not a JSON object".into())
            })?;
            for key in SECRET_ENV_KEYS {
                if env.remove(*key).is_some() {
                    stripped = true;
                }
            }
        }
        // Codex 供应商的 `auth` 对象是 auth.json 的镜像，`OPENAI_API_KEY`
        // 住在里面——同样受密钥清单约束，剥离规则与 `env` 一致：非对象
        // auth 无法证明密钥缺失，宁可不发布。
        if let Some(auth) = obj.get_mut("auth") {
            let auth = auth.as_object_mut().ok_or_else(|| {
                AppError::Config("provider settingsConfig auth is not a JSON object".into())
            })?;
            for key in SECRET_ENV_KEYS {
                if auth.remove(*key).is_some() {
                    stripped = true;
                }
            }
        }
        // Template variables recorded in meta (raw JSON text, frontend-owned)
        // can carry the same secret keys — strip them too. Unparseable meta
        // never proves them absent.
        let meta_trimmed = self.meta.trim();
        let mut meta: serde_json::Value = if meta_trimmed.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(meta_trimmed)
                .map_err(|e| AppError::Config(format!("provider meta is not valid JSON: {e}")))?
        };
        let mut meta_stripped = false;
        if let Some(values) = meta
            .get_mut("templateValues")
            .and_then(|tv| tv.as_object_mut())
        {
            for key in SECRET_ENV_KEYS {
                if values.remove(*key).is_some() {
                    meta_stripped = true;
                }
            }
            // Stripped everything ⇒ drop the now-empty record instead of
            // publishing `{"templateValues":{}}` noise.
            if meta_stripped && values.is_empty() {
                if let Some(meta_obj) = meta.as_object_mut() {
                    meta_obj.remove("templateValues");
                }
            }
        }
        if !stripped && !meta_stripped {
            return Ok(self.clone());
        }
        let mut p = self.clone();
        if stripped {
            p.settings_config = serde_json::to_string_pretty(&v)?;
        }
        if meta_stripped {
            p.meta = serde_json::to_string_pretty(&meta)?;
        }
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
        assert_eq!(App::default(), App::Claude);
        assert_eq!(App::Claude.as_str(), "claude");
        assert_eq!(App::Codex.as_str(), "codex");
        assert_eq!(App::Gemini.as_str(), "gemini");
    }

    #[test]
    fn app_unknown_db_str_falls_back_to_claude() {
        assert_eq!(App::from_db_str("bogus"), App::Claude);
        assert_eq!(App::from_db_str(""), App::Claude);
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

    /// TEMP-APP-SHIM 语义：无 app 字段的旧同步文件 / 导出文档读为 claude
    ///（#32 迁移后同样语义）；带 app 字段的新文档按值解析。
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
