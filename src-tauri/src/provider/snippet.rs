//! 通用配置片段（Common Config Snippet）：一段跨供应商共享的 settings.json
//! 片段（默认 `{"includeCoAuthoredBy": false}`），勾选启用后切换写盘时合并进
//! 受控字段。
//!
//! 合并语义（纯函数，测试直接覆盖）：
//! - **片段只落合并域声明受控的位置**（[`MergeDomain`]——per-app 受控区形状，
//!   分派在 `live_adapter` 的 [`App::snippet_layer`]）：claude = env 子键 +
//!   `CONTROLLED_FIELDS` 顶层开关（清单外顶层键一律忽略——片段绝不污染非受控
//!   字段）；gemini = env 子键 + settings.json 顶层整体（声明即生效——机制
//!   承载力，名册另由校验约束为 env-only，见 [`MergeDomain::WholeTopLevel`]）。
//!   `merge_live_settings` 是第二道闸，这里是第一道。
//! - **合并方向：片段是共享默认值，供应商显式配置优先**。
//!   - `env` 键级深合并：供应商 env 已有的键保留供应商的值，片段 env 只补充
//!     供应商缺失的键。
//!   - 顶层：供应商已配置则保留，未配置时片段的值补上（范围由合并域声明）。
//! - `apply_snippet` 是写盘入口：片段未启用 → 原样返回（不解析、不合并）；
//!   启用 → 按调用方给的合并域合并。「取消勾选后下次写盘不再合并」这个验收
//!   不变量落在可测的纯函数里，而不是命令薄壳里。
//!
//! 存储：片段**按应用各一份**（claude / codex / gemini / grok），存本机
//! config.json（`ConfigData::common_config_snippets`，存取走
//! [`ConfigData::snippet_for`] / [`ConfigData::set_snippet`]；存量单条已迁移
//! 到 claude 键），与激活状态同属本机配置——config.json 从不进 git、不随
//! 同步仓库走，因此本模块不碰任何 sync 文件。片段校验与合并的**按应用分派**
//! 收口在 `live_adapter` 的 [`App::validate_snippet`] /
//! [`App::merge_extracted_snippet`]（per-app 行为单一 seam），本模块只负责
//! 合并 / 校验纯函数本身（claude / gemini 的 JSON 侧校验：
//! [`validate_claude_snippet`] / [`validate_gemini_snippet`]；codex / grok 的
//! TOML 侧在各自 live_* 模块）。
//!
//! 校验：`validate_claude_snippet` / `validate_gemini_snippet` 供 set 命令经
//! seam 调用——非法 JSON 或非对象 → `Err`；空/纯空白合法（合并时视为 `{}`，
//! 即无操作）。写盘时启用的片段解析不了 → `Err`（切换失败）：宁可显式失败，
//! 也不静默丢片段效果。

use crate::error::{AppError, AppResult};
use crate::provider::live::{parse_object, CONTROLLED_FIELDS};
use crate::provider::settings_codec::{ENV_FIELD, GOOGLE_GEMINI_BASE_URL_ENV};

/// 片段合并域（merge 的受控区形状入参——per-app，由 `live_adapter` 的
/// [`App::snippet_layer`] 随 settings_config 层一起声明）：「片段允许落进
/// 供应商 settings_config 的哪些位置」。两种域的 env 块同做键级补缺失；差异
/// 在顶层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeDomain {
    /// 白名单域（claude，ADR-0005 受控字段）：顶层只补 [`CONTROLLED_FIELDS`]
    /// 内的开关（env 除外——env 恒走键级合并）；清单外顶层键一律忽略。
    ControlledFields,
    /// 顶层整体域（gemini，ADR-0010：受控区 = settings.json 顶层整体，声明即
    /// 接管）：片段声明的顶层键补缺失进 settings_config。**这是机制承载力，
    /// 不是名册**——片段校验（ADR-0010 名册决策「Gemini 片段 = JSON env 对象」）
    /// 仍只放行 env 子对象，顶层键经 set 拦截根本进不了合并；「名册放宽 =
    /// 改校验清单即生效」的能力就此存在，放宽与否另行决策。
    WholeTopLevel,
}

/// TOML 片段整理（「整理」按钮）：用 taplo（保留注释的规范 formatter，VS Code
/// Even Better TOML 同引擎）把压缩文本展开成多行。**保留注释与键序**——默认
/// `reorder_keys: false`；taplo 解析时跳过语法错误区（不因残片而失败，与 JSON
/// 侧 formatJson 的容错一致）。薄壳：读入 → taplo 格式化。
pub fn format_toml(text: &str) -> String {
    taplo::formatter::format(text, taplo::formatter::Options::default())
}

/// 写盘入口（纯函数）：片段启用 → 按 `domain`（该 app 的受控区形状）把片段
/// 合并进 settingsConfig；未启用 → 原样返回（不解析片段——停用的片段没有
/// 任何效果）。合并域由调用方按 app 给出（`live_adapter` 的
/// [`App::snippet_layer`] 随 settings_config 层一起声明）。
pub fn apply_snippet(
    settings_config: &str,
    snippet: &str,
    enabled: bool,
    domain: MergeDomain,
) -> AppResult<String> {
    if !enabled {
        return Ok(settings_config.to_string());
    }
    merge_snippet_into_settings(settings_config, snippet, domain)
}

/// 合并纯函数：把片段落进 `domain` 声明的受控区（供应商显式配置优先），不碰
/// 文件系统。输出是合并后的 settingsConfig JSON 文本（2 空格缩进，与
/// [`live::merge_live_settings`] 的清洗输出一致）。
///
/// 写盘层兜底：片段携带凭据键 → `Err`——claude/gemini 的凭据拦截不只在 set
/// 时（[`validate_claude_snippet`] / [`validate_gemini_snippet`]，经 seam 分派），
/// 绕过 set 直接走合并/写盘的路径也拦（与 codex/grok 的 `merge_*_snippet`
/// 双层同构：set 拦 + 合并纯函数兜底）。
///
/// 边界：`snippet` 为空串/纯空白 → 视为 `{}`（启用但没写内容 = 无操作）；
/// `snippet` 非法 JSON 或非对象 → `Err`；`settings_config` 非法 JSON 或非对象
/// → `Err`（合并需要解析它；非法的配置本来也过不了写盘）。
pub fn merge_snippet_into_settings(
    settings_config: &str,
    snippet: &str,
    domain: MergeDomain,
) -> AppResult<String> {
    let mut target = parse_object(settings_config, "provider settingsConfig")?;
    let snippet_obj = parse_snippet_or_empty(snippet)?;
    reject_sensitive_keys(&snippet_obj, "claude/gemini")?;
    let snippet_map = snippet_obj.as_object();

    let target_obj = target.as_object_mut().expect("parsed object");

    // env 键级深合并（两种域同语义）：供应商显式配置优先，片段只补缺失的键。
    // 片段 env 非对象（手写垃圾）→ 跳过合并，供应商 env 原样保留。
    if let Some(snippet_env) = snippet_obj.get(ENV_FIELD).and_then(|v| v.as_object()) {
        let target_env = target_obj
            .entry(ENV_FIELD.to_string())
            .or_insert_with(|| serde_json::json!({}));
        if let Some(target_env_obj) = target_env.as_object_mut() {
            for (key, value) in snippet_env {
                if !target_env_obj.contains_key(key) {
                    target_env_obj.insert(key.clone(), value.clone());
                }
            }
        }
    }

    // 顶层按合并域补缺失：供应商已配置 → 保留；缺失 → 片段补上。env 已按
    // 键级合并，整键跳过（片段 env 不得整块覆盖供应商 env）。
    match domain {
        MergeDomain::ControlledFields => {
            // 白名单域：非受控键根本不在 CONTROLLED_FIELDS 里，天然被忽略。
            for key in CONTROLLED_FIELDS {
                if *key == ENV_FIELD {
                    continue;
                }
                if !target_obj.contains_key(*key) {
                    if let Some(value) = snippet_obj.get(*key) {
                        target_obj.insert((*key).to_string(), value.clone());
                    }
                }
            }
        }
        MergeDomain::WholeTopLevel => {
            // 顶层整体域：片段声明的一切顶层键声明即生效（ADR-0010 gemini
            // 受控区）。当前名册下片段只可能是 env 子对象（校验拦截其余），
            // 故生产行为与「只认 env」等价——这里是机制承载力。
            if let Some(map) = snippet_map {
                for (key, value) in map {
                    if key == ENV_FIELD {
                        continue;
                    }
                    if !target_obj.contains_key(key) {
                        target_obj.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    Ok(serde_json::to_string_pretty(&target)?)
}

/// claude 片段校验（set 命令经 `live_adapter` seam 调用，分派见
/// [`App::validate_snippet`]）：合法 JSON 对象（空串=空片段）；拒绝凭据键
/// （env 是认证通道，见 ADR-0010）。
pub(crate) fn validate_claude_snippet(snippet: &str) -> AppResult<()> {
    let obj = parse_snippet_or_empty(snippet)?;
    reject_sensitive_keys(&obj, "claude")
}

/// gemini 片段校验（set 命令经 `live_adapter` seam 调用）：只认 `env` 子对象
/// （名册决策——ADR-0010「Gemini 片段 = JSON env 对象」；合并域虽能承载顶层
/// 键，名册放宽另行决策）、拒凭据键与端点键
/// [`GOOGLE_GEMINI_BASE_URL_ENV`]、要求 env 值为非空字符串。
pub(crate) fn validate_gemini_snippet(snippet: &str) -> AppResult<()> {
    let obj = parse_snippet_or_empty(snippet)?;
    // gemini 片段只认 env 子对象：这是名册决策而非机制约束（合并域 = 顶层
    // 整体，机制能承载顶层键）——但在名册收紧的前提下，放行顶层键 = 用户
    // 以为配好了实际不进片段语义，明确拒绝并指因。
    if let Some(map) = obj.as_object() {
        for key in map.keys() {
            if key != ENV_FIELD {
                return Err(AppError::Config(format!(
                    "gemini 通用片段只认 env 子对象，顶层键 `{key}` 不在片段名册（请写进 {{\"env\":{{...}}}}）"
                )));
            }
        }
    }
    reject_sensitive_keys(&obj, "gemini")?;
    validate_gemini_extras(&obj)
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
/// [`GOOGLE_GEMINI_BASE_URL_ENV`]（`GEMINI_API_KEY` 已被凭据模式 `_API_KEY`
/// 覆盖）。端点键决定凭据发往何处，共享会把认证引到错误端点。
fn validate_gemini_extras(obj: &serde_json::Value) -> AppResult<()> {
    let Some(env) = obj.get("env").and_then(|v| v.as_object()) else {
        return Ok(()); // 无 env 子对象 = 无可校验的值
    };
    for (key, value) in env {
        if key == GOOGLE_GEMINI_BASE_URL_ENV {
            return Err(AppError::Config(format!(
                "gemini 通用片段不得包含端点键 {GOOGLE_GEMINI_BASE_URL_ENV}（端点键归供应商）"
            )));
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
/// **不**用本函数（允许 `mcp_servers` 含凭据），只禁身份键。`import_live` 反向
/// 导入也用它判定「片段候选」里剔除凭据键（前后端各一镜像，ADR-0010）。
/// 凭据键精确匹配表（转大写后全等）。模块级化（原函数体内私有 const）以便
/// security parity 测试从同一份权威组装 fixture——表本身仍只有这一处。
pub(crate) const SENSITIVE_EXACT: &[&str] = &[
    "APIKEY",
    "API_KEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "CREDENTIALS",
];

/// 凭据键后缀表（转大写后以此结尾）。
pub(crate) const SENSITIVE_SUFFIXES: &[&str] = &[
    "_KEY",
    "_API_KEY",
    "_ACCESS_KEY",
    "_ACCESS_KEY_ID",
    "_KEY_ID",
    "_PRIVATE_KEY",
    "_APIKEY",
    "_ACCESSKEY",
    "_SECRETKEY",
    "_APITOKEN",
    "_AUTH_TOKEN",
    "_TOKEN",
    "_PAT",
    "_PWD",
    "_PASS",
    "_PASSPHRASE",
    "_CREDS",
];

/// 凭据键子串表（转大写后包含）。
pub(crate) const SENSITIVE_CONTAINS: &[&str] = &[
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "CREDENTIAL",
    "PRIVATE_KEY",
    "BEARER_TOKEN",
];

pub(crate) fn is_sensitive_config_key(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SENSITIVE_EXACT.iter().any(|e| &upper == e)
        || SENSITIVE_SUFFIXES.iter().any(|s| upper.ends_with(s))
        || SENSITIVE_CONTAINS.iter().any(|c| upper.contains(c))
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
    use MergeDomain::{ControlledFields, WholeTopLevel};

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
        let out = parsed(&merge_snippet_into_settings(cfg, snippet, ControlledFields).unwrap());
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
        let out = parsed(
            &merge_snippet_into_settings("{}", r#"{"env":{"A":"1"}}"#, ControlledFields).unwrap(),
        );
        assert_eq!(out["env"], serde_json::json!({"A": "1"}));
    }

    #[test]
    fn snippet_controlled_switch_fills_when_provider_missing() {
        let out = parsed(
            &merge_snippet_into_settings(
                r#"{"env":{}}"#,
                r#"{"includeCoAuthoredBy": false, "attribution": "default"}"#,
                ControlledFields,
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
        let out = parsed(&merge_snippet_into_settings(cfg, snippet, ControlledFields).unwrap());
        assert_eq!(
            out["includeCoAuthoredBy"],
            serde_json::json!(true),
            "供应商显式配置优先，片段是共享默认值"
        );
    }

    #[test]
    fn snippet_non_controlled_keys_are_ignored() {
        // 白名单域：片段里塞了非受控键——必须被忽略，绝不进入合并结果。
        let snippet = r#"{
            "includeCoAuthoredBy": false,
            "permissions": {"deny": ["Bash"]},
            "hooks": {"PostToolUse": [{"matcher": "*"}]},
            "mcpServers": {"filesystem": {"command": "npx"}},
            "model": "claude-opus-4-5",
            "enableAllProjectMcpServers": true,
            "statusLine": {"type": "command", "command": "echo hi"}
        }"#;
        let out = parsed(
            &merge_snippet_into_settings(r#"{"env":{}}"#, snippet, ControlledFields).unwrap(),
        );
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
        let out = parsed(
            &merge_snippet_into_settings(
                r#"{"env":{}}"#,
                r#"{"env": "garbage"}"#,
                ControlledFields,
            )
            .unwrap(),
        );
        assert_eq!(out["env"], serde_json::json!({}), "垃圾 env 不合并");
    }

    #[test]
    fn empty_snippet_is_a_noop() {
        let cfg = r#"{"env": {"A": "1"}, "includeCoAuthoredBy": false}"#;
        for empty in ["", "   ", "\n"] {
            let out = parsed(&merge_snippet_into_settings(cfg, empty, ControlledFields).unwrap());
            assert_eq!(out["env"], serde_json::json!({"A": "1"}));
            assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
        }
    }

    #[test]
    fn invalid_snippet_json_is_an_error() {
        let r = merge_snippet_into_settings(r#"{"env":{}}"#, "{nope", ControlledFields);
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn non_object_snippet_is_an_error() {
        for snippet in [r#"[1,2,3]"#, r#""just a string""#] {
            let r = merge_snippet_into_settings(r#"{"env":{}}"#, snippet, ControlledFields);
            assert!(
                matches!(r, Err(AppError::Config(_))),
                "非对象片段必须失败: {snippet}"
            );
        }
    }

    #[test]
    fn invalid_settings_config_is_an_error() {
        let r = merge_snippet_into_settings("{nope", r#"{"env":{}}"#, ControlledFields);
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn apply_snippet_disabled_passes_through_unchanged() {
        // 未启用：不解析不合并，原样返回——停用的片段（哪怕是垃圾文本）不
        // 能影响写盘内容。
        let cfg = r#"{"env": {"A": "1"}}"#;
        let out = apply_snippet(cfg, "totally {not json", false, ControlledFields).unwrap();
        assert_eq!(out, cfg);
    }

    #[test]
    fn apply_snippet_enabled_merges() {
        let out = apply_snippet(
            r#"{"env": {"A": "1"}}"#,
            r#"{"env": {"B": "2"}, "includeCoAuthoredBy": false}"#,
            true,
            ControlledFields,
        )
        .unwrap();
        let v = parsed(&out);
        assert_eq!(v["env"], serde_json::json!({"A": "1", "B": "2"}));
        assert_eq!(v["includeCoAuthoredBy"], serde_json::json!(false));
    }

    /// gemini 的顶层整体域与 claude 白名单域在 env-only 片段下行为等价
    /// （红线：参数化不改变 gemini 现状行为——当前名册下片段只可能是 env
    /// 子对象）。
    #[test]
    fn gemini_whole_top_level_env_only_behavior_equals_claude_domain() {
        let cfg = r#"{"env":{"GEMINI_MODEL":"mine"}}"#;
        let snippet = r#"{"env":{"GEMINI_MODEL":"snippet?","GEMINI_EXTRA":"y"}}"#;
        let as_gemini = merge_snippet_into_settings(cfg, snippet, WholeTopLevel).unwrap();
        let as_claude = merge_snippet_into_settings(cfg, snippet, ControlledFields).unwrap();
        assert_eq!(
            parsed(&as_gemini),
            parsed(&as_claude),
            "env-only 片段两域等价"
        );
        let v = parsed(&as_gemini);
        assert_eq!(v["env"]["GEMINI_MODEL"], "mine", "供应商赢");
        assert_eq!(v["env"]["GEMINI_EXTRA"], "y", "缺失键补上");
    }

    /// gemini 顶层整体域的机制承载力：片段顶层键补缺失进 settingsConfig
    /// （「声明即生效」的能力存在；供应商已声明的顶层键仍供应商赢）。但名册
    /// （validate_gemini_snippet，ADR-0010「Gemini 片段 = JSON env 对象」）仍
    /// 只放行 env 子对象——顶层键经 set 拦截根本进不了合并；名册放宽另行
    /// 决策，本测只锁机制能力本身。
    #[test]
    fn gemini_whole_top_level_domain_fills_top_level_keys() {
        let cfg = r#"{"env":{},"selectedTheme":"dark","model":"vendor-model"}"#;
        let snippet = r#"{"env":{"GEMINI_MODEL":"m"},"mcpServers":{"fs":{"command":"npx"}},"selectedTheme":"auto"}"#;
        let out = parsed(&merge_snippet_into_settings(cfg, snippet, WholeTopLevel).unwrap());
        assert_eq!(
            out["mcpServers"]["fs"]["command"],
            serde_json::json!("npx"),
            "顶层新键声明即生效"
        );
        assert_eq!(
            out["selectedTheme"], "dark",
            "供应商已声明的顶层键保留（供应商赢）"
        );
        assert_eq!(
            out["model"], "vendor-model",
            "供应商显式配置优先（片段是共享默认值）"
        );
        assert_eq!(out["env"]["GEMINI_MODEL"], "m", "env 键级补缺失两域同语义");
        // 同一片段在 claude 白名单域下：非受控顶层键被忽略（对照）。
        let claude_out =
            parsed(&merge_snippet_into_settings(cfg, snippet, ControlledFields).unwrap());
        assert!(claude_out.get("mcpServers").is_none());
    }

    #[test]
    fn merged_snippet_flows_through_live_write_path() {
        // 复用写盘路径：合并后的 settingsConfig 交给 merge_live_settings，
        // live 的非受控字段仍原地保留、受控字段被片段 + 供应商内容覆盖。
        // 片段 env 不带凭据键（写盘层兜底拒绝，见
        // write_layer_rejects_credential_keys_outside_set_path）。
        let live = live_with_uncontrolled();
        let snippet_cfg = merge_snippet_into_settings(
            r#"{"env": {"ANTHROPIC_BASE_URL": "https://x.dev"}}"#,
            r#"{"env": {"ANTHROPIC_SMALL_FAST_MODEL": "haiku"}, "includeCoAuthoredBy": false}"#,
            ControlledFields,
        )
        .unwrap();
        let out = parsed(&merge_live_settings(&live, &snippet_cfg).unwrap());
        assert_eq!(
            out["env"],
            serde_json::json!({
                "ANTHROPIC_BASE_URL": "https://x.dev",
                "ANTHROPIC_SMALL_FAST_MODEL": "haiku"
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

    /// 写盘层兜底：凭据拦截不只在 set（App::validate_snippet），绕过 set 直接
    /// 合并的路径也拒（与 codex/grok 的 merge_*_snippet 双层同构）。两个域
    /// 同一凭据拦截（域只管键落点，不管凭据）。
    #[test]
    fn write_layer_rejects_credential_keys_outside_set_path() {
        for domain in [ControlledFields, WholeTopLevel] {
            let r = merge_snippet_into_settings(
                r#"{"env":{}}"#,
                r#"{"env": {"ANTHROPIC_AUTH_TOKEN": "sk-x"}}"#,
                domain,
            );
            assert!(
                matches!(r, Err(AppError::Config(_))),
                "合并纯函数必须拒绝凭据键（写盘层兜底，域 {domain:?}）"
            );
            let r2 = apply_snippet(r#"{"env":{}}"#, r#"{"apiKey": "x"}"#, true, domain);
            assert!(
                matches!(r2, Err(AppError::Config(_))),
                "apply_snippet 写盘入口同样拒绝顶层凭据键（域 {domain:?}）"
            );
        }
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

    #[test]
    fn format_toml_expands_compressed_text_to_multiline() {
        // 压缩成一行/少行的 TOML 展开成多行（taplo 保格式）。
        let src = r#"[tui]
theme = "dark""#;
        let out = format_toml(src);
        assert!(out.contains("theme = \"dark\""), "展开后保留键值: {out}");
        assert!(out.lines().count() > 1, "展开成多行: {out}");
    }

    #[test]
    fn format_toml_preserves_comments() {
        // 注释必须保留——这是选 taplo 而非 smol-toml 的原因。
        let src = r#"[tui]
# 用户注释
theme = "dark""#;
        let out = format_toml(src);
        assert!(out.contains("# 用户注释"), "注释保留: {out}");
    }

    #[test]
    fn format_toml_does_not_reorder_keys() {
        // TOML 不排序（reorder_keys 默认 false）——与 JSON 排序相反，避免重排
        // 用户刻意安排的键序。
        let src = r#"[a]
z = 1
m = 2"#;
        let out = format_toml(src);
        assert!(
            out.find("z").unwrap() < out.find("m").unwrap(),
            "键序保持: {out}"
        );
    }

    #[test]
    fn format_toml_tolerates_syntax_errors() {
        // taplo「跳过语法错误区」：残片也返回字符串，不抛错（与 formatJson 容错一致）。
        let out = format_toml("z = [1, 2");
        assert!(!out.trim().is_empty());
    }
}
