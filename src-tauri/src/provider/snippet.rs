//! 通用配置片段（Common Config Snippet）：一段跨供应商共享的 settings.json
//! 片段（默认 `{"includeCoAuthoredBy": false}`），勾选启用后切换写盘时合并进
//! 受控字段。
//!
//! 合并语义（纯函数，测试直接覆盖）：
//! - **片段只允许受控字段**（[`live::CONTROLLED_FIELDS`]）：片段里出现非受控
//!   键（permissions / hooks / mcpServers / model 等）一律忽略——片段绝不
//!   污染 live 的非受控字段。`merge_live_settings` 是第二道闸，这里是第一道。
//! - **合并方向：片段是共享默认值，供应商显式配置优先**。
//!   - `env` 键级深合并：供应商 env 已有的键保留供应商的值，片段 env 只补充
//!     供应商缺失的键。
//!   - 其余受控顶层开关（`includeCoAuthoredBy` / `attribution` / ...）：
//!     供应商已配置则保留，未配置时片段的值补上。
//! - `apply_snippet` 是写盘入口：片段未启用 → 原样返回（不解析、不合并）；
//!   启用 → 合并。「取消勾选后下次写盘不再合并」这个验收不变量落在可测的
//!   纯函数里，而不是命令薄壳里。
//!
//! 存储：片段**按应用各一份**（claude / codex / gemini），存本机
//! config.json（`ConfigData::common_config_snippets`，存取走
//! [`ConfigData::snippet_for`] / [`ConfigData::set_snippet`]；存量单条已迁移
//! 到 claude 键），与激活状态同属本机配置——config.json 从不进 git、不随
//! 同步仓库走，因此本模块不碰任何 sync 文件。写盘合并按应用分派（codex /
//! gemini 的合并语义）归后续批次，本模块只负责合并纯函数本身。
//!
//! 校验：`validate_snippet` 供 set 命令用——非法 JSON 或非对象 → `Err`；
//! 空/纯空白合法（合并时视为 `{}`，即无操作）。写盘时启用的片段解析不了 →
//! `Err`（切换失败）：宁可显式失败，也不静默丢片段效果。

use crate::error::{AppError, AppResult};
use crate::model::App;
use crate::provider::live::{parse_object, CONTROLLED_FIELDS};

/// 写盘入口（纯函数）：片段启用 → 把片段合并进 settingsConfig；未启用 →
/// 原样返回（不解析片段——停用的片段没有任何效果）。
pub fn apply_snippet(settings_config: &str, snippet: &str, enabled: bool) -> AppResult<String> {
    if !enabled {
        return Ok(settings_config.to_string());
    }
    merge_snippet_into_settings(settings_config, snippet)
}

/// 合并纯函数：把片段的**受控字段**合并进供应商 settingsConfig，不碰文件
/// 系统。输出是合并后的 settingsConfig JSON 文本（2 空格缩进，与
/// [`live::merge_live_settings`] 的清洗输出一致）。
///
/// 边界：`snippet` 为空串/纯空白 → 视为 `{}`（启用但没写内容 = 无操作）；
/// `snippet` 非法 JSON 或非对象 → `Err`；`settings_config` 非法 JSON 或非对象
/// → `Err`（合并需要解析它；非法的配置本来也过不了写盘）。
pub fn merge_snippet_into_settings(settings_config: &str, snippet: &str) -> AppResult<String> {
    let mut target = parse_object(settings_config, "provider settingsConfig")?;
    let snippet_obj = parse_snippet_or_empty(snippet)?;

    let target_obj = target.as_object_mut().expect("parsed object");

    // env 键级深合并：供应商显式配置优先，片段只补缺失的键。片段 env 非对象
    // （手写垃圾）→ 跳过合并，供应商 env 原样保留。
    if let Some(snippet_env) = snippet_obj.get("env").and_then(|v| v.as_object()) {
        let target_env = target_obj
            .entry("env".to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(target_env_obj) = target_env.as_object_mut() {
            for (key, value) in snippet_env {
                if !target_env_obj.contains_key(key) {
                    target_env_obj.insert(key.clone(), value.clone());
                }
            }
        }
    }

    // 其余受控顶层开关：供应商已配置 → 保留；缺失 → 片段补上。非受控键
    // 根本不在 CONTROLLED_FIELDS 里，天然被忽略。
    for key in CONTROLLED_FIELDS {
        if *key == "env" {
            continue;
        }
        if !target_obj.contains_key(*key) {
            if let Some(value) = snippet_obj.get(*key) {
                target_obj.insert((*key).to_string(), value.clone());
            }
        }
    }

    Ok(serde_json::to_string_pretty(&target)?)
}

/// 片段校验（set 命令用，按应用分派）：
/// - claude / gemini：合法 JSON 对象（空串=空片段）；拒绝凭据键（env 是认证
///   通道，见 ADR-0010）；gemini 另拒端点键 `GOOGLE_GEMINI_BASE_URL`、要求
///   env 值为非空字符串。
/// - codex / grok：合法 TOML；拒绝受控身份键（凭据键不禁——`mcp_servers` 不
///   经 LLM 端点）。
/// - opencode：附加模式无片段概念 → `Ok`。
pub fn validate_snippet(app: App, snippet: &str) -> AppResult<()> {
    match app {
        App::Claude => {
            let obj = parse_snippet_or_empty(snippet)?;
            reject_sensitive_keys(&obj, "claude")
        }
        App::Gemini => {
            let obj = parse_snippet_or_empty(snippet)?;
            reject_sensitive_keys(&obj, "gemini")?;
            validate_gemini_extras(&obj)
        }
        App::Codex => crate::provider::live_codex::validate_codex_snippet(snippet),
        App::Grok => crate::provider::live_grok::validate_grok_snippet(snippet),
        App::OpenCode => Ok(()),
    }
}

/// 扫描 JSON 片段的键（顶层 + env 子对象）是否含凭据模式键（
/// [`is_sensitive_config_key`]）→ `Err`。claude / gemini 共用：两者片段都写 env
/// （认证通道），凭据进片段会随请求发往供应商端点。非凭据键（模型/端点/开关）
/// 照常放行——cc one「供应商赢」下供应商自带值顶掉片段，无需像 CC-Switch
/// （片段赢）那样维护供应商专属键剥离清单。
fn reject_sensitive_keys(obj: &serde_json::Value, app: &str) -> AppResult<()> {
    let Some(map) = obj.as_object() else {
        return Ok(());
    };
    for key in map.keys() {
        if is_sensitive_config_key(key) {
            return Err(AppError::Config(format!(
                "{app} 通用片段不得包含凭据键 `{key}`（env 是认证通道，见 ADR-0010）"
            )));
        }
    }
    if let Some(env) = map.get("env").and_then(|v| v.as_object()) {
        for key in env.keys() {
            if is_sensitive_config_key(key) {
                return Err(AppError::Config(format!(
                    "{app} 通用片段不得包含凭据键 `env.{key}`（env 是认证通道，见 ADR-0010）"
                )));
            }
        }
    }
    Ok(())
}

/// gemini 片段额外校验：env 值必须是非空字符串；另拒端点键
/// `GOOGLE_GEMINI_BASE_URL`（`GEMINI_API_KEY` 已被凭据模式 `_API_KEY` 覆盖）。
/// 端点键决定凭据发往何处，共享会把认证引到错误端点。
fn validate_gemini_extras(obj: &serde_json::Value) -> AppResult<()> {
    let Some(env) = obj.get("env").and_then(|v| v.as_object()) else {
        return Ok(()); // 无 env 子对象 = 无可校验的值
    };
    for (key, value) in env {
        if key == "GOOGLE_GEMINI_BASE_URL" {
            return Err(AppError::Config(
                "gemini 通用片段不得包含端点键 GOOGLE_GEMINI_BASE_URL（端点键归供应商）".into(),
            ));
        }
        let s = value.as_str().ok_or_else(|| {
            AppError::Config(format!("gemini 通用片段 env.{key} 的值必须是字符串"))
        })?;
        if s.trim().is_empty() {
            return Err(AppError::Config(format!(
                "gemini 通用片段 env.{key} 的值不得为空"
            )));
        }
    }
    Ok(())
}

/// 判定一个配置键名是否像凭据（API key / token / secret / password 等）。
///
/// 用模式匹配覆盖整类凭据键，而非枚举具体名字——枚举挡不住下一个 `*_API_KEY`
/// （CC-Switch 曾因只枚举两个固定键名，让 `GOOGLE_API_KEY` 漏进共享片段、被
/// 深合并进其它供应商 env、随请求发往对方 base_url）。三张表移植自 CC-Switch
/// `is_sensitive_config_key`，**前端 TS 镜像（#50 gemini 落地时）须逐字一致**
/// （否则后端拦了、前端还能写回）：EXACT 精确、SUFFIXES 后缀、CONTAINS 子串；
/// 统一转大写后比较。
///
/// Claude / Gemini 片段校验共用——两者都写 env（认证通道），凭据进片段会随请求
/// 发往供应商端点。codex / grok 片段写的是 `mcp_servers` 等（不经 LLM 端点），
/// **不**用本函数（允许 `mcp_servers` 含凭据），只禁身份键。仅本模块内调用，
/// TS 镜像落地前无需对外暴露。
fn is_sensitive_config_key(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "APIKEY", "API_KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIALS",
    ];
    const SUFFIXES: &[&str] = &[
        "_KEY", "_API_KEY", "_ACCESS_KEY", "_ACCESS_KEY_ID", "_KEY_ID", "_PRIVATE_KEY",
        "_APIKEY", "_ACCESSKEY", "_SECRETKEY", "_APITOKEN", "_AUTH_TOKEN", "_TOKEN",
        "_PAT", "_PWD", "_PASS", "_PASSPHRASE", "_CREDS",
    ];
    const CONTAINS: &[&str] = &[
        "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "PRIVATE_KEY", "BEARER_TOKEN",
    ];
    let upper = name.to_ascii_uppercase();
    EXACT.iter().any(|e| &upper == e)
        || SUFFIXES.iter().any(|s| upper.ends_with(s))
        || CONTAINS.iter().any(|c| upper.contains(c))
}

/// 解析片段：空串/纯空白 → `{}`；非法 JSON 或非对象 → `Err`。
fn parse_snippet_or_empty(snippet: &str) -> AppResult<serde_json::Value> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    parse_object(trimmed, "common config snippet")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::provider::live::merge_live_settings;

    fn parsed(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    /// 一份带非受控字段的 live 配置（模拟用户手动配了 hooks / permissions 的
    /// settings.json），供「合并结果流经写盘路径」测试用。
    fn live_with_uncontrolled() -> String {
        r#"{
  "env": {"ANTHROPIC_MODEL": "live-model"},
  "permissions": {"allow": ["Bash"]},
  "hooks": {"PreToolUse": [{"matcher": "Bash"}]}
}"#
        .to_string()
    }

    #[test]
    fn snippet_env_fills_missing_keys_and_provider_wins() {
        let cfg = r#"{"env": {"ANTHROPIC_MODEL": "m1", "KEEP_ME": "1"}}"#;
        let snippet = r#"{"env": {"ANTHROPIC_MODEL": "snippet-wins?", "ANTHROPIC_BASE_URL": "https://x.dev"}}"#;
        let out = parsed(&merge_snippet_into_settings(cfg, snippet).unwrap());
        assert_eq!(
            out["env"],
            serde_json::json!({
                "ANTHROPIC_MODEL": "m1",
                "KEEP_ME": "1",
                "ANTHROPIC_BASE_URL": "https://x.dev"
            }),
            "供应商显式配置优先：同键保留供应商值，片段只补缺失键"
        );
    }

    #[test]
    fn snippet_env_added_when_provider_has_no_env() {
        let out = parsed(&merge_snippet_into_settings("{}", r#"{"env":{"A":"1"}}"#).unwrap());
        assert_eq!(out["env"], serde_json::json!({"A": "1"}));
    }

    #[test]
    fn snippet_controlled_switch_fills_when_provider_missing() {
        let out = parsed(
            &merge_snippet_into_settings(
                r#"{"env":{}}"#,
                r#"{"includeCoAuthoredBy": false, "attribution": "default"}"#,
            )
            .unwrap(),
        );
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
        assert_eq!(out["attribution"], serde_json::json!("default"));
    }

    #[test]
    fn provider_explicit_value_wins_over_snippet_switch() {
        let cfg = r#"{"includeCoAuthoredBy": true}"#;
        let snippet = r#"{"includeCoAuthoredBy": false}"#;
        let out = parsed(&merge_snippet_into_settings(cfg, snippet).unwrap());
        assert_eq!(
            out["includeCoAuthoredBy"],
            serde_json::json!(true),
            "供应商显式配置优先，片段是共享默认值"
        );
    }

    #[test]
    fn snippet_non_controlled_keys_are_ignored() {
        // 片段里塞了非受控键——必须被忽略，绝不进入合并结果。
        let snippet = r#"{
            "includeCoAuthoredBy": false,
            "permissions": {"deny": ["Bash"]},
            "hooks": {"PostToolUse": [{"matcher": "*"}]},
            "mcpServers": {"filesystem": {"command": "npx"}},
            "model": "claude-opus-4-5",
            "enableAllProjectMcpServers": true,
            "statusLine": {"type": "command", "command": "echo hi"}
        }"#;
        let out = parsed(&merge_snippet_into_settings(r#"{"env":{}}"#, snippet).unwrap());
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
        assert!(out.get("permissions").is_none(), "permissions 被忽略");
        assert!(out.get("hooks").is_none(), "hooks 被忽略");
        assert!(out.get("mcpServers").is_none(), "mcpServers 被忽略");
        assert!(out.get("model").is_none(), "model 被忽略");
        assert!(
            out.get("enableAllProjectMcpServers").is_none(),
            "enableAllProjectMcpServers 被忽略"
        );
        assert!(out.get("statusLine").is_none(), "statusLine 被忽略");
    }

    #[test]
    fn snippet_non_object_env_is_ignored() {
        let out =
            parsed(&merge_snippet_into_settings(r#"{"env":{}}"#, r#"{"env": "garbage"}"#).unwrap());
        assert_eq!(out["env"], serde_json::json!({}), "垃圾 env 不合并");
    }

    #[test]
    fn empty_snippet_is_a_noop() {
        let cfg = r#"{"env": {"A": "1"}, "includeCoAuthoredBy": false}"#;
        for empty in ["", "   ", "\n"] {
            let out = parsed(&merge_snippet_into_settings(cfg, empty).unwrap());
            assert_eq!(out["env"], serde_json::json!({"A": "1"}));
            assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
        }
    }

    #[test]
    fn invalid_snippet_json_is_an_error() {
        let r = merge_snippet_into_settings(r#"{"env":{}}"#, "{nope");
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn non_object_snippet_is_an_error() {
        for snippet in [r#"[1,2,3]"#, r#""just a string""#] {
            let r = merge_snippet_into_settings(r#"{"env":{}}"#, snippet);
            assert!(
                matches!(r, Err(AppError::Config(_))),
                "非对象片段必须失败: {snippet}"
            );
        }
    }

    #[test]
    fn invalid_settings_config_is_an_error() {
        let r = merge_snippet_into_settings("{nope", r#"{"env":{}}"#);
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn apply_snippet_disabled_passes_through_unchanged() {
        // 未启用：不解析不合并，原样返回——停用的片段（哪怕是垃圾文本）不
        // 能影响写盘内容。
        let cfg = r#"{"env": {"A": "1"}}"#;
        let out = apply_snippet(cfg, "totally {not json", false).unwrap();
        assert_eq!(out, cfg);
    }

    #[test]
    fn apply_snippet_enabled_merges() {
        let out = apply_snippet(
            r#"{"env": {"A": "1"}}"#,
            r#"{"env": {"B": "2"}, "includeCoAuthoredBy": false}"#,
            true,
        )
        .unwrap();
        let v = parsed(&out);
        assert_eq!(v["env"], serde_json::json!({"A": "1", "B": "2"}));
        assert_eq!(v["includeCoAuthoredBy"], serde_json::json!(false));
    }

    #[test]
    fn merged_snippet_flows_through_live_write_path() {
        // 复用写盘路径：合并后的 settingsConfig 交给 merge_live_settings，
        // live 的非受控字段仍原地保留、受控字段被片段 + 供应商内容覆盖。
        let live = live_with_uncontrolled();
        let snippet_cfg = merge_snippet_into_settings(
            r#"{"env": {"ANTHROPIC_BASE_URL": "https://x.dev"}}"#,
            r#"{"env": {"ANTHROPIC_AUTH_TOKEN": "sk-x"}, "includeCoAuthoredBy": false}"#,
        )
        .unwrap();
        let out = parsed(&merge_live_settings(&live, &snippet_cfg, &[]).unwrap());
        assert_eq!(
            out["env"],
            serde_json::json!({
                "ANTHROPIC_BASE_URL": "https://x.dev",
                "ANTHROPIC_AUTH_TOKEN": "sk-x"
            })
        );
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
        assert_eq!(
            out["permissions"],
            serde_json::json!({"allow": ["Bash"]}),
            "live 的非受控字段保留"
        );
        assert_eq!(
            out["hooks"],
            serde_json::json!({"PreToolUse": [{"matcher": "Bash"}]})
        );
    }

    #[test]
    fn validate_snippet_accepts_object_and_empty() {
        assert!(validate_snippet(App::Claude, r#"{"includeCoAuthoredBy": false}"#).is_ok());
        assert!(validate_snippet(App::Claude, "").is_ok());
        assert!(validate_snippet(App::Claude, "   ").is_ok());
    }

    #[test]
    fn validate_snippet_rejects_invalid_and_non_object() {
        assert!(matches!(
            validate_snippet(App::Claude, "{nope"),
            Err(AppError::Config(_))
        ));
        assert!(matches!(
            validate_snippet(App::Claude, r#"[1]"#),
            Err(AppError::Config(_))
        ));
    }

    #[test]
    fn validate_claude_snippet_rejects_credentials() {
        // env 里的凭据键拒绝（env 是认证通道）。
        assert!(validate_snippet(
            App::Claude,
            r#"{"env": {"ANTHROPIC_AUTH_TOKEN": "sk-x"}}"#
        )
        .is_err());
        assert!(validate_snippet(
            App::Claude,
            r#"{"env": {"ANTHROPIC_API_KEY": "sk-x"}}"#
        )
        .is_err());
        // 顶层凭据键也拒。
        assert!(validate_snippet(App::Claude, r#"{"apiKey": "x"}"#).is_err());
        // 非凭据键放行（模型/端点/开关——供应商赢下无害）。
        assert!(validate_snippet(
            App::Claude,
            r#"{"env": {"ANTHROPIC_MODEL": "m", "ANTHROPIC_BASE_URL": "u"}}"#
        )
        .is_ok());
    }

    #[test]
    fn validate_gemini_snippet_rejects_credentials_endpoint_and_empty() {
        // 凭据键拒绝（GEMINI_API_KEY 命中 _API_KEY）。
        assert!(validate_snippet(
            App::Gemini,
            r#"{"env": {"GEMINI_API_KEY": "k"}}"#
        )
        .is_err());
        // 端点键拒绝。
        assert!(validate_snippet(
            App::Gemini,
            r#"{"env": {"GOOGLE_GEMINI_BASE_URL": "u"}}"#
        )
        .is_err());
        // env 值非字符串拒绝。
        assert!(validate_snippet(App::Gemini, r#"{"env": {"GEMINI_MODEL": 123}}"#).is_err());
        // env 值空串拒绝。
        assert!(validate_snippet(App::Gemini, r#"{"env": {"GEMINI_MODEL": "  "}}"#).is_err());
        // 合法：非凭据、非端点、非空字符串。
        assert!(validate_snippet(
            App::Gemini,
            r#"{"env": {"GEMINI_MODEL": "gemini-2.5-flash"}}"#
        )
        .is_ok());
    }

    #[test]
    fn validate_codex_and_grok_snippet_delegate_identity_rejection() {
        // codex：身份键拒绝（TOML），非受控键放行（含 mcp_servers 凭据）。
        assert!(validate_snippet(App::Codex, r#"model = "x""#).is_err());
        assert!(validate_snippet(
            App::Codex,
            r#"[mcp_servers.github]
command = "npx"
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_x" }"#
        )
        .is_ok());
        // grok：身份键拒绝（cc-one profile / models.default）。
        assert!(validate_snippet(
            App::Grok,
            r#"[model.cc-one]
model = "x""#
        )
        .is_err());
        assert!(validate_snippet(
            App::Grok,
            r#"[mcp_servers.github]
command = "npx""#
        )
        .is_ok());
    }

    #[test]
    fn sensitive_keys_detected() {
        // 凭据键：token / key / secret / password 各类形态。
        for key in [
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY", // CC-Switch 泄漏事故的同款键
            "APIKEY",
            "API_KEY",
            "TOKEN",
            "SECRET",
            "OPENAI_API_KEY",
            "MY_PRIVATE_KEY",
            "DB_PASSWORD",
            "SERVICE_ACCOUNT_CREDENTIALS",
        ] {
            assert!(is_sensitive_config_key(key), "{key} 应判为凭据");
        }
    }

    #[test]
    fn non_sensitive_keys_pass_through() {
        // 非凭据键：模型名、端点、功能开关——必须放行（用户真实 env 里绝大多数
        // 是这类，取自真实 settings.json）。
        for key in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "CLAUDE_CODE_EFFORT_LEVEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_ATTRIBUTION_HEADER",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "GEMINI_MODEL",
            "MY_FLAG",
        ] {
            assert!(!is_sensitive_config_key(key), "{key} 不应判为凭据");
        }
    }

    #[test]
    fn sensitive_detection_is_case_insensitive() {
        assert!(is_sensitive_config_key("anthropic_auth_token"));
        assert!(is_sensitive_config_key("OpenAI_Api_Key"));
        assert!(!is_sensitive_config_key("anthropic_model"));
    }
}
