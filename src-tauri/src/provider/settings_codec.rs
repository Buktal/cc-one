//! settings_config 形状编解码（per-app 单一事实来源）。
//!
//! 「app X 的 settings_config 里住着什么」此前没有 Rust 侧单源：写侧 parse
//! 散在 `live_codex` / `live_gemini` / `live_grok` / `live` 四个模块、读侧
//! construct 手拼在 `import_live` 与 `import_ccswitch`、密钥键名又一份拼写
//! 活在 `keys.rs`——六处 `OPENAI_API_KEY`、`GOOGLE_GEMINI_BASE_URL` 裸写
//! 字面量都是这份无主知识的漂移面。本模块把形状收成一份：字段名 / 键名常量
//! （唯一拼写处，调用方一律引用常量）+ typed 值 ⇄ settings_config 文本的
//! build / parse 双向纯函数，build 与 parse 互为镜像（往返由本模块测试钉住）。
//!
//! 四个单激活 app 的形状（数据本身，不是散文）：
//! - claude：受控子集对象本体——`{env, 顶层开关}`（[`CONTROLLED_FIELDS`]），
//!   settings.json 是字面量 JSON，无包装字段；
//! - codex：`{"auth":{"OPENAI_API_KEY"},"config":"<TOML>"}`；
//! - gemini：`{"env":{字符串键值},"config":{settings.json 顶层声明}}`（env 值
//!   须字符串——`.env` 只能表达键值）；
//! - grok：`{"config":"<TOML>"}`（cc-one profile 块）。
//!
//! 分派形态：所有调用方都是静态已知的 app（写盘流各自只处理自己的 app、导入
//! 反向解析按 app 各一函数），故不做运行时 enum 分派——本模块按 app 分节的
//! 命名函数即分派单源，加 app 时从 `live_adapter` 的穷尽 match 出发自然落到
//! 这里补一份 build / parse。

use std::collections::HashMap;

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::provider::live::{
    config_toml_field, parse_and_strip_settings, parse_target_or_empty, strip_internal_keys,
};

// ---------------- 键名 / 字段名（唯一拼写处）----------------------------

/// claude / gemini settings_config 装 env 键值块的顶层字段名。
pub(crate) const ENV_FIELD: &str = "env";
/// codex settings_config 装 auth 对象的顶层字段名（镜像 `auth.json`）。
pub(crate) const CODEX_AUTH_FIELD: &str = "auth";
/// codex auth 对象里的密钥键名：写盘写它、反向解析读它、同步剥它——三处
/// 曾各拼一遍，现全部引用本常量。
pub(crate) const CODEX_AUTH_SECRET_KEY: &str = "OPENAI_API_KEY";
/// codex / grok settings_config 装 config TOML 文本的顶层字段名。
pub(crate) const CONFIG_TOML_FIELD: &str = "config";
/// gemini settings_config 装 settings.json 顶层声明的顶层字段名。
pub(crate) const GEMINI_CONFIG_FIELD: &str = "config";
/// gemini env 块的 API Key 键：env 含它 → API Key 版（写盘 selectedType 判定），
/// 同步剥密钥的精确键名。
pub(crate) const GEMINI_API_KEY_ENV: &str = "GEMINI_API_KEY";
/// gemini env 块的端点键：端点决定凭据发往何处——归供应商接管，永不进共享
/// 片段（提取排除、片段校验拒绝、导入候选同判都引用本常量）。
pub(crate) const GOOGLE_GEMINI_BASE_URL_ENV: &str = "GOOGLE_GEMINI_BASE_URL";

// ---------------- typed 值（四 app 各一形状）----------------------------

/// codex 的 typed settings_config：auth 密钥（`None` = 登录态版，写盘不碰
/// auth.json）+ config TOML 文本。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CodexSettings {
    /// trim 后非空才有效；`None` / 空 → 不写 auth.json。
    pub auth_key: Option<String>,
    /// 目标 config.toml 文本（受控合并的 target；空串 = 无受控内容）。
    pub config_toml: String,
}

/// gemini 的 typed settings_config：`env`（整块写 `.env`，值须字符串）+
/// `config`（settings.json 顶层受控区的声明——声明的顶层键整体替换；`None` =
/// 无声明替换，身份键撤除清单仍生效）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GeminiSettings {
    pub env: HashMap<String, String>,
    pub config: Option<serde_json::Map<String, Value>>,
}

// ---------------- build（typed 值 → settings_config 文本）----------------

/// claude（build 半向）：env 键值对 → `{"env":{...}}`。受控顶层开关目前没有
/// construct 场景（开关只见于 live 反向解析的受控子集拷贝）；需要时扩本函数
/// 入参，不在调用方手拼第二份形状。
pub(crate) fn build_claude_settings(env: impl IntoIterator<Item = (String, String)>) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert(
        ENV_FIELD.to_string(),
        Value::Object(
            env.into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        ),
    );
    to_pretty_json(&Value::Object(obj))
}

/// codex（build 半向）：auth 密钥 + config TOML →
/// `{"auth":{...},"config":"..."}`。密钥 `None` → 空 auth 对象（登录态版形状，
/// parse 半向视为无 key）；`Some` 的值**原样写入**（不 trim——写盘侧的 trim 是
/// `parse_codex_settings` 的语义，调用方要判空自己先判）。
pub(crate) fn build_codex_settings(auth_key: Option<&str>, config_toml: &str) -> String {
    let mut auth = serde_json::Map::new();
    if let Some(key) = auth_key {
        auth.insert(
            CODEX_AUTH_SECRET_KEY.to_string(),
            Value::String(key.to_string()),
        );
    }
    let mut obj = serde_json::Map::new();
    obj.insert(CODEX_AUTH_FIELD.to_string(), Value::Object(auth));
    obj.insert(
        CONFIG_TOML_FIELD.to_string(),
        Value::String(config_toml.to_string()),
    );
    to_pretty_json(&Value::Object(obj))
}

/// gemini（build 半向）：env 键值 + settings.json 顶层声明 →
/// `{"env":{...},"config":{...}}`。config `None` → 不带 config 字段。env 值
/// 必须是字符串（`.env` 只能表达键值）——调用方类型即约束。
pub(crate) fn build_gemini_settings(
    env: impl IntoIterator<Item = (String, String)>,
    config: Option<serde_json::Map<String, Value>>,
) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert(
        ENV_FIELD.to_string(),
        Value::Object(
            env.into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
        ),
    );
    if let Some(config) = config {
        obj.insert(GEMINI_CONFIG_FIELD.to_string(), Value::Object(config));
    }
    to_pretty_json(&Value::Object(obj))
}

/// grok（build 半向）：cc-one profile 块 TOML 文本 → `{"config":"<TOML>"}`。
pub(crate) fn build_grok_settings(config_toml: &str) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert(
        CONFIG_TOML_FIELD.to_string(),
        Value::String(config_toml.to_string()),
    );
    to_pretty_json(&Value::Object(obj))
}

/// `Value` → pretty JSON 文本。`serde_json::Value` 的全部变体都是合法 JSON，
/// 序列化不会失败——`expect` 是类型事实，不是乐观假设。
fn to_pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("serde_json Value is always serializable")
}

// ---------------- parse（settings_config 文本 → typed 值）----------------

/// claude（parse 半向）：settings_config 文本 → 受控子集对象。空串/纯空白 →
/// `{}`；非法 JSON / 非对象 → `Err`；剥内部 meta 键；`env`（若带）必须为对象
/// ——env 是受控字段、写盘时整块替换，非对象（手写/导入的坏配置）会被原样
/// 带进用户 settings.json，宁可报错阻止写盘。
pub(crate) fn parse_claude_settings(settings_config: &str) -> AppResult<Value> {
    let mut target = parse_target_or_empty(settings_config)?;
    if let Some(obj) = target.as_object_mut() {
        strip_internal_keys(obj);
    }
    if let Some(env) = target.get(ENV_FIELD) {
        if !env.is_object() {
            return Err(AppError::Config(
                "provider settingsConfig env is not a JSON object".into(),
            ));
        }
    }
    Ok(target)
}

/// codex（parse 半向）：提取 `auth.<CODEX_AUTH_SECRET_KEY>`（trim 非空才有，
/// 其余登录态字段不进 typed 值）与 `config` TOML。
///
/// 边界：空串/纯空白 → 空载荷（无 key 无 config）；非对象 settingsConfig、
/// 非对象 `auth`、非字符串密钥键（空串除外，视为无 key）、非字符串 `config`
/// → `Err`（坏配置不能进用户 auth.json / config.toml）。
pub(crate) fn parse_codex_settings(settings_config: &str) -> AppResult<CodexSettings> {
    let Some(obj) = parse_and_strip_settings(settings_config)? else {
        return Ok(CodexSettings::default());
    };
    let auth_key = match obj.get(CODEX_AUTH_FIELD) {
        None => None,
        Some(auth) => {
            let auth_obj = auth.as_object().ok_or_else(|| {
                AppError::Config(format!(
                    "provider settingsConfig {CODEX_AUTH_FIELD} is not a JSON object"
                ))
            })?;
            match auth_obj.get(CODEX_AUTH_SECRET_KEY) {
                None => None,
                Some(key) => {
                    let key = key.as_str().ok_or_else(|| {
                        AppError::Config(format!(
                            "provider settingsConfig {CODEX_AUTH_FIELD}.{CODEX_AUTH_SECRET_KEY} must be a string"
                        ))
                    })?;
                    let key = key.trim();
                    if key.is_empty() {
                        None
                    } else {
                        Some(key.to_string())
                    }
                }
            }
        }
    };
    let config_toml = config_toml_field(&obj)?;
    Ok(CodexSettings {
        auth_key,
        config_toml,
    })
}

/// gemini（parse 半向）：settingsConfig 文本 → [`GeminiSettings`]（校验见各
/// 分支；写盘 `write_gemini_live_at` 的第一步就是它）。
///
/// 边界：settingsConfig 必须是 JSON 对象；`env` 必须缺失或是对象且值均为
/// 字符串（静默丢键会让认证悄悄坏掉，宁可失败）；`config` 必须缺失、是对象
/// 或 `null`（`null` 与缺失同义）；顶层剥掉内部 meta 字段。
pub(crate) fn parse_gemini_settings(raw: &str) -> AppResult<GeminiSettings> {
    let mut value = parse_target_or_empty(raw)?;
    let obj = value
        .as_object_mut()
        .expect("parse_target_or_empty yields object");
    strip_internal_keys(obj);

    let env = match obj.remove(ENV_FIELD) {
        None => HashMap::new(),
        Some(v) => {
            let map = v.as_object().ok_or_else(|| {
                AppError::Config("provider settingsConfig env is not a JSON object".into())
            })?;
            let mut out = HashMap::new();
            for (key, value) in map {
                let s = value.as_str().ok_or_else(|| {
                    AppError::Config(format!(
                        "provider settingsConfig env value for {key} is not a string"
                    ))
                })?;
                out.insert(key.clone(), s.to_string());
            }
            out
        }
    };

    let config = match obj.remove(GEMINI_CONFIG_FIELD) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let map = v.as_object().ok_or_else(|| {
                AppError::Config(
                    "provider settingsConfig config is not a JSON object or null".into(),
                )
            })?;
            Some(map.clone())
        }
    };

    Ok(GeminiSettings { env, config })
}

/// grok（parse 半向）：settings_config（`{"config":"<TOML>"}`）→ 目标 TOML
/// 文本。空串/纯空白 → 空目标（登录态版）；非对象 settingsConfig、非字符串
/// `config` → `Err`（坏配置不能进用户 config.toml）。
pub(crate) fn parse_grok_settings(settings_config: &str) -> AppResult<String> {
    let Some(obj) = parse_and_strip_settings(settings_config)? else {
        return Ok(String::new());
    };
    config_toml_field(&obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::keys;

    fn parsed(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    // ---- build ⇄ parse 往返（每 app 一条；形状文本只有本模块一份声明）----

    #[test]
    fn claude_build_parse_round_trips() {
        let env = [
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.moonshot.cn/anthropic".to_string(),
            ),
            ("ANTHROPIC_MODEL".to_string(), "kimi-k2".to_string()),
        ];
        let text = build_claude_settings(env.clone());
        let parsed = parse_claude_settings(&text).unwrap();
        assert_eq!(
            parsed.get(ENV_FIELD),
            Some(&Value::Object(
                env.into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect()
            )),
            "env 块往返语义不变: {text}"
        );
        // claude 的受控子集即对象本体：build 产物除 env 外无其它包装字段。
        let obj = parsed.as_object().unwrap();
        assert_eq!(obj.len(), 1, "build 只声明 env，无第二处形状: {text}");
    }

    #[test]
    fn codex_build_parse_round_trips() {
        let typed = CodexSettings {
            auth_key: Some("sk-codex-x".to_string()),
            config_toml: "model = \"gpt-5.6\"\n".to_string(),
        };
        let text = build_codex_settings(typed.auth_key.as_deref(), &typed.config_toml);
        assert_eq!(parse_codex_settings(&text).unwrap(), typed, "{text}");

        // 登录态版：无 key → 空 auth 对象，parse 半向视为无 key。
        let login = build_codex_settings(None, "");
        let back = parse_codex_settings(&login).unwrap();
        assert_eq!(back.auth_key, None);
        assert_eq!(back.config_toml, "");
        assert_eq!(parsed(&login)[CODEX_AUTH_FIELD], serde_json::json!({}));
    }

    #[test]
    fn gemini_build_parse_round_trips() {
        let env = [
            (GEMINI_API_KEY_ENV.to_string(), "sk-gem-x".to_string()),
            (
                GOOGLE_GEMINI_BASE_URL_ENV.to_string(),
                "https://gen.dev".to_string(),
            ),
            ("GEMINI_MODEL".to_string(), "gemini-2.5-flash".to_string()),
        ];
        let config = parsed(r#"{"model":"gemini-2.5-flash"}"#)
            .as_object()
            .unwrap()
            .clone();
        let typed = GeminiSettings {
            env: env.clone().into_iter().collect(),
            config: Some(config),
        };
        let text = build_gemini_settings(typed.env.clone(), typed.config.clone());
        assert_eq!(parse_gemini_settings(&text).unwrap(), typed, "{text}");

        // 无 config 声明：build 不带该字段，parse 半向为 None。
        let no_config = build_gemini_settings(env, None);
        let back = parse_gemini_settings(&no_config).unwrap();
        assert_eq!(back.config, None, "config 缺席 = 无声明替换");
        assert!(parsed(&no_config).get(GEMINI_CONFIG_FIELD).is_none());
    }

    #[test]
    fn grok_build_parse_round_trips() {
        let toml = "[model.cc-one]\nmodel = \"grok-4.5\"\n";
        let text = build_grok_settings(toml);
        assert_eq!(parse_grok_settings(&text).unwrap(), toml, "{text}");
        assert_eq!(
            parsed(&text)[CONFIG_TOML_FIELD],
            serde_json::json!(toml),
            "config 是 TOML 字符串字段"
        );
    }

    /// 键名漂移守护（build ⇄ 密钥清洗两处键名的红绿灯）：codec build 写出的
    /// 密钥键必须被 `keys.rs` 的位置清单剥掉——两处键名曾各拼各的、靠人眼同步
    /// （sync 侧漏剥 auth 位置是真实缺陷），现在键名常量编译期同源 + 这条
    /// 往返断言双保险。
    #[test]
    fn built_secret_keys_are_stripped_by_keys_manifest() {
        let codex = build_codex_settings(Some("sk-codex-secret"), "");
        let stripped = keys::strip_settings_config(&codex, "provider").unwrap();
        assert!(
            !stripped.contains("sk-codex-secret"),
            "codex auth 密钥必须被同步剥除: {stripped}"
        );

        let gemini_env = [
            (
                GEMINI_API_KEY_ENV.to_string(),
                "sk-gemini-secret".to_string(),
            ),
            ("GEMINI_MODEL".to_string(), "m".to_string()),
        ];
        let gemini = build_gemini_settings(gemini_env, None);
        let stripped = keys::strip_settings_config(&gemini, "provider").unwrap();
        assert!(
            !stripped.contains("sk-gemini-secret"),
            "gemini env 密钥必须被同步剥除: {stripped}"
        );
        assert!(stripped.contains("GEMINI_MODEL"), "非密钥 env 键保留");
    }

    // ---- codex parse（写盘流的第一步，自 live_codex 移居于此）----

    #[test]
    fn codex_parse_extracts_key_and_config() {
        let s = parse_codex_settings(
            r#"{"auth":{"OPENAI_API_KEY":" sk-123 "},"config":"model = \"m\""}"#,
        )
        .unwrap();
        assert_eq!(s.auth_key.as_deref(), Some("sk-123"), "key 要 trim");
        assert_eq!(s.config_toml, r#"model = "m""#);
    }

    #[test]
    fn codex_parse_login_state_versions_have_no_key() {
        // 官方登录态版：空 auth / 空 key / 无 auth 字段 → 都不写 auth.json。
        for raw in [
            r#"{"auth":{}}"#,
            r#"{"auth":{"OPENAI_API_KEY":""}}"#,
            r#"{"auth":{"OPENAI_API_KEY":"   "}}"#,
            r#"{"config":"model = \"m\""}"#,
            "{}",
        ] {
            let s = parse_codex_settings(raw).unwrap();
            assert_eq!(s.auth_key, None, "登录态版无 key: {raw}");
        }
    }

    #[test]
    fn codex_parse_strips_internal_meta_keys() {
        let s = parse_codex_settings(
            r#"{"api_format":"openai","apiFormat":"openai","openrouter_compat_mode":true,"openrouterCompatMode":true,"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        )
        .unwrap();
        assert_eq!(s.auth_key.as_deref(), Some("sk-1"));
        assert_eq!(s.config_toml, r#"model = "m""#);
    }

    #[test]
    fn codex_parse_rejects_bad_shapes() {
        // 非对象 settingsConfig。
        assert!(parse_codex_settings("[1,2]").is_err());
        assert!(parse_codex_settings(r#""just a string""#).is_err());
        // 非对象 auth。
        assert!(parse_codex_settings(r#"{"auth":"sk-plain"}"#).is_err());
        // 非字符串密钥键。
        assert!(parse_codex_settings(r#"{"auth":{"OPENAI_API_KEY":123}}"#).is_err());
        // 非字符串 config。
        assert!(parse_codex_settings(r#"{"config":123}"#).is_err());
    }

    #[test]
    fn codex_parse_empty_settings_is_an_empty_snapshot() {
        for raw in ["", "   "] {
            let s = parse_codex_settings(raw).unwrap();
            assert_eq!(s.auth_key, None);
            assert_eq!(s.config_toml, "");
        }
    }

    // ---- gemini parse（自 live_gemini 移居于此）----

    #[test]
    fn gemini_parse_extracts_env_and_config() {
        let s = parse_gemini_settings(
            r#"{"env":{"GEMINI_API_KEY":"sk-x","GEMINI_MODEL":"m"},"config":{"model":"m"}}"#,
        )
        .unwrap();
        assert_eq!(
            s.env.get(GEMINI_API_KEY_ENV).map(String::as_str),
            Some("sk-x")
        );
        assert_eq!(s.env.get("GEMINI_MODEL").map(String::as_str), Some("m"));
        let config = s.config.expect("config parsed");
        assert_eq!(config.get("model"), Some(&serde_json::json!("m")));
    }

    #[test]
    fn gemini_parse_accepts_missing_env_and_null_config() {
        let s = parse_gemini_settings(r#"{"config": null}"#).unwrap();
        assert!(s.env.is_empty());
        assert!(s.config.is_none());
        let s2 = parse_gemini_settings(r#"{"env": {}}"#).unwrap();
        assert!(s2.env.is_empty());
        assert!(s2.config.is_none());
        // 空串 → 空目标（写空 env + oauth 标记）。
        let s3 = parse_gemini_settings("").unwrap();
        assert!(s3.env.is_empty() && s3.config.is_none());
    }

    #[test]
    fn gemini_parse_strips_internal_meta_keys_from_top_level() {
        let s = parse_gemini_settings(
            r#"{"api_format":"gemini","apiFormat":"gemini","openrouter_compat_mode":true,"env":{"GEMINI_API_KEY":"k"}}"#,
        )
        .unwrap();
        assert_eq!(s.env.get(GEMINI_API_KEY_ENV).map(String::as_str), Some("k"));
        // 内部键不进 config、不进 env——它们只供应用内部读。
        assert!(s.config.is_none());
    }

    #[test]
    fn gemini_parse_rejects_invalid_json_and_non_object() {
        for bad in ["{oops", r#"[1,2,3]"#, r#""str""#] {
            assert!(
                parse_gemini_settings(bad).is_err(),
                "非法/非对象 settingsConfig 必须失败: {bad}"
            );
        }
    }

    #[test]
    fn gemini_parse_rejects_non_object_env_and_non_string_values() {
        for bad in [r#"{"env":"garbage"}"#, r#"{"env":[1]}"#] {
            assert!(
                parse_gemini_settings(bad).is_err(),
                "env 非对象必须失败: {bad}"
            );
        }
        // 值非字符串（数字/对象）：.env 只能表达键值，静默丢键会让认证悄悄
        // 坏掉，宁可失败。
        assert!(parse_gemini_settings(r#"{"env":{"GEMINI_API_KEY":123}}"#).is_err());
        assert!(parse_gemini_settings(r#"{"env":{"GEMINI_API_KEY":{"a":1}}}"#).is_err());
    }

    #[test]
    fn gemini_parse_rejects_config_that_is_neither_object_nor_null() {
        for bad in [r#"{"config":123}"#, r#"{"config":"x"}"#, r#"{"config":[]}"#] {
            assert!(
                parse_gemini_settings(bad).is_err(),
                "config 非对象非 null 必须失败: {bad}"
            );
        }
    }

    // ---- claude parse（merge_live_settings 的目标侧校验）----

    #[test]
    fn claude_parse_accepts_empty_and_strips_internal_keys() {
        assert_eq!(
            parse_claude_settings("  ").unwrap(),
            serde_json::json!({}),
            "空串 → 空目标"
        );
        let parsed = parse_claude_settings(
            r#"{"api_format":"x","apiFormat":"x","openrouter_compat_mode":true,"openrouterCompatMode":true,"env":{"ANTHROPIC_MODEL":"m"}}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.get(ENV_FIELD),
            Some(&serde_json::json!({"ANTHROPIC_MODEL":"m"})),
            "内部 meta 键剥除、env 保留"
        );
    }

    #[test]
    fn claude_parse_rejects_bad_shapes_and_non_object_env() {
        for bad in ["{oops", r#"[1,2,3]"#, r#""just a string""#] {
            assert!(parse_claude_settings(bad).is_err(), "{bad}");
        }
        // env 非对象（手写/导入的坏配置）会原样写进用户 settings.json，拒绝。
        for bad in [r#"{"env": "garbage"}"#, r#"{"env": ["A=1"]}"#] {
            assert!(parse_claude_settings(bad).is_err(), "{bad}");
        }
    }

    // ---- grok parse（自 live_grok 移居于此）----

    #[test]
    fn grok_parse_extracts_config_toml() {
        let s = parse_grok_settings(r#"{"config":"[model.cc-one]\nmodel = \"m\""}"#).unwrap();
        assert!(s.contains("[model.cc-one]"));
        assert!(s.contains(r#"model = "m""#));
    }

    #[test]
    fn grok_parse_strips_internal_meta_keys() {
        let s = parse_grok_settings(
            r#"{"api_format":"openai","apiFormat":"openai","openrouter_compat_mode":true,"config":"[model.cc-one]\nmodel = \"m\""}"#,
        )
        .unwrap();
        assert!(s.contains("[model.cc-one]"));
    }

    #[test]
    fn grok_parse_rejects_bad_shapes() {
        assert!(parse_grok_settings("[1,2]").is_err());
        assert!(parse_grok_settings(r#""just a string""#).is_err());
        assert!(parse_grok_settings(r#"{"config":123}"#).is_err());
    }

    #[test]
    fn grok_parse_empty_settings_is_empty_target() {
        for raw in ["", "   "] {
            assert_eq!(parse_grok_settings(raw).unwrap(), "");
        }
        // 无 config 字段也视为空目标。
        assert_eq!(parse_grok_settings("{}").unwrap(), "");
    }
}
