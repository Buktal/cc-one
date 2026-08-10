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

use crate::error::AppResult;
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

/// 片段校验（set 命令用）：空/纯空白合法；非法 JSON 或非对象 → `Err`。
/// 片段内容原样保存，不做规范化——编辑器直接编辑原文。
pub fn validate_snippet(snippet: &str) -> AppResult<()> {
    let _ = parse_snippet_or_empty(snippet)?;
    Ok(())
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
        assert!(validate_snippet(r#"{"includeCoAuthoredBy": false}"#).is_ok());
        assert!(validate_snippet("").is_ok());
        assert!(validate_snippet("   ").is_ok());
    }

    #[test]
    fn validate_snippet_rejects_invalid_and_non_object() {
        assert!(matches!(
            validate_snippet("{nope"),
            Err(AppError::Config(_))
        ));
        assert!(matches!(
            validate_snippet(r#"[1]"#),
            Err(AppError::Config(_))
        ));
    }
}
