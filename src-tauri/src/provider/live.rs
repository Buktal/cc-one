//! Provider 写盘（live）：受控合并 + 备份 + 原子写。
//!
//! 写盘分派点 `write_live(app, provider)`（claude / codex / gemini 三分支）
//! 见本文件下方；codex 分支的 TOML 合并 + auth.json 在 `live_codex` 模块。
//! 以下写盘语义是 claude 分支（JSON 受控合并）的精确规格，codex/gemini 各自
//! 沿用同一套「受控合并 / 非受控保留 / 备份 / 原子写」语义：
//!
//! 写盘语义（必须精确实现）：
//! - **受控字段**（Provider 接管，切换时整块替换/合并）：`env` 块 +
//!   `includeCoAuthoredBy` / `attribution` / `effortLevel` / `enabledPlugins` /
//!   `skipWebFetchPreflight`。`env` 走整块替换（端点/key/模型映射都住在 env
//!   里），其余顶层开关按「目标存在则替换、缺失则保留 live 原值」合并。
//! - **非受控字段**（`permissions` / `hooks` / `mcpServers` /
//!   `enableAllProjectMcpServers` / `model` / `extraKnownMarketplaces` /
//!   `statusLine` 等一切其他字段）：切换时从 live **原地保留**；目标配置里的
//!   非受控字段被忽略，绝不写 live。
//! - 写盘顺序：读当前 live → 受控合并 → 备份 `settings.json.bak`（单份覆盖）→
//!   原子写（临时文件 + 改名，进程中断不产生半截文件）。
//! - 清洗：写 live 前剥掉配置里的应用内部 meta 字段（`api_format` /
//!   `apiFormat` 等，类比 cc-switch `sanitize_claude_settings_for_live`）。
//! - **不做** cc-switch 的整文件覆盖 + Backfill。
//!
//! `merge_live_settings` 是纯函数（本项目最高价值的测试接缝）：输入
//! (当前 live JSON 字符串, 目标 settingsConfig 字符串, 清洗规则) → 输出合并后的
//! JSON 字符串，不碰文件系统。文件 IO（读/备份/原子写）是薄壳，直接调用它。
//! 「非受控字段保留」这个关键不变量靠它落进可测代码，而不是散文注释。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};

/// 写盘时从配置里剥掉的内部 meta 字段（类比 cc-switch
/// `sanitize_claude_settings_for_live`）：这些键只供应用自己读，不是合法的
/// settings.json 字段，绝不落 live。
pub const LIVE_INTERNAL_KEYS: &[&str] = &[
    "api_format",
    "apiFormat",
    "openrouter_compat_mode",
    "openrouterCompatMode",
];

/// 受控字段：切换时整块替换/合并。除这些键之外的任何字段
/// （permissions / hooks / mcpServers / ...）都不是受控字段，切换时一律从
/// live 原地保留。
pub const CONTROLLED_FIELDS: &[&str] = &[
    "env",
    "includeCoAuthoredBy",
    "attribution",
    "effortLevel",
    "enabledPlugins",
    "skipWebFetchPreflight",
];

/// Claude Code 用户级 settings.json 路径（跨平台统一 `~/.claude/settings.json`）。
pub fn claude_settings_path() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".claude").join("settings.json"))
}

/// Merge 纯函数（测试接缝 1）：把目标 settingsConfig 的**受控字段**合并进当前
/// live 配置，非受控字段从 live 原样保留，不碰文件系统。
///
/// 语义：
/// - `env` 整块替换：目标有 `env`（哪怕是空对象）→ live 的 env 被整体覆盖；
///   目标没有 `env` → live 的 env 原样保留。
/// - 其余受控顶层开关：目标存在则替换，缺失则保留 live 原值。
/// - 非受控字段：一律从 live 保留；目标里的非受控字段被忽略（绝不写 live，
///   否则一切换就清空用户手动的 hooks / MCP / permissions）。
/// - 清洗：合并前剥掉目标里的内部字段（`internal_keys`），合并后再对结果剥
///   一遍（live 里若残留旧应用的内部键也一并清掉）。
///
/// 边界：`live` 为空串/纯空白 → 视为 `{}`（没有现存配置可保留）；`live` 是
/// 非空非法 JSON 或非对象 → `Err`（解析不了就没法保留用户手动配置，宁可失败）；
/// `target` 为空串 → 视为 `{}`；`target` 非法 JSON、非对象、或 `env` 非对象
/// → `Err`（坏配置不能进用户 settings.json）。
pub fn merge_live_settings(live: &str, target: &str, internal_keys: &[&str]) -> AppResult<String> {
    let mut merged = parse_live_or_empty(live)?;
    let mut target_obj = parse_target_or_empty(target)?;

    // 清洗目标：剥内部 meta 字段，防止它们被当作受控字段带进 live。
    if let Some(obj) = target_obj.as_object_mut() {
        for key in internal_keys {
            obj.remove(*key);
        }
    }

    // 目标 `env` 必须是对象：env 是受控字段，写盘时整块替换 live 的 env——
    // 非对象（手写/导入的坏配置）会被原样带进用户 settings.json。宁可报错
    // 阻止写盘，与「目标非法 JSON 报错」同一原则：配置坏了就显式失败。
    if let Some(env) = target_obj.get("env") {
        if !env.is_object() {
            return Err(AppError::Config(
                "provider settingsConfig env is not a JSON object".into(),
            ));
        }
    }

    // 受控合并：只从目标提取受控字段，其余一律忽略。
    let merged_obj = merged.as_object_mut().expect("merged is always an object");
    for key in CONTROLLED_FIELDS {
        if let Some(value) = target_obj.get(*key) {
            merged_obj.insert((*key).to_string(), value.clone());
        }
    }

    // 清洗结果：live 里残留的内部键也剥掉，保证写出去的 live 永远不含它们。
    if let Some(obj) = merged.as_object_mut() {
        for key in internal_keys {
            obj.remove(*key);
        }
    }

    Ok(serde_json::to_string_pretty(&merged)?)
}

/// 读当前 live settings.json；文件不存在 → 空串（merge 视为 `{}`）。
pub fn read_live_settings(path: &Path) -> AppResult<String> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// 把当前 live 备份为 `<name>.<ext>.bak`（单份覆盖）。live 不存在时跳过——
/// 没有可备份的内容。claude `settings.json` 与 codex `config.toml` 共用同一
/// 条备份规则（备份路径由文件扩展名推导，见 [`backup_path`]）。
pub fn backup_live_settings(path: &Path) -> AppResult<()> {
    backup_file(path)
}

/// 通用备份（写盘前调用）：目标存在才备份，`.bak` 单份覆盖不堆积。
/// codex config.toml 的写前备份走这里（auth.json 是凭据/登录态，不备份）。
pub(crate) fn backup_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    fs::copy(path, backup_path(path))?;
    Ok(())
}

/// 备份路径：`settings.json` → `settings.json.bak`，`config.toml` →
/// `config.toml.bak`（保留原名，追加 `.bak` 到扩展名之后）。
pub(crate) fn backup_path(path: &Path) -> PathBuf {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_extension(format!("{ext}.bak"))
}

/// 原子写：先把内容写入同目录的临时文件（独立名字，避免并发写冲突），再改名
/// 覆盖目标。进程在写盘中途中断只会留下临时文件，不会产生半截 live 文件。
/// claude settings.json 与 codex config.toml/auth.json 共用（单一事实来源）。
pub(crate) fn atomic_write_file(path: &Path, content: &str) -> AppResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("live file path has no parent dir".into()))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("live file path has no file name".into()))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!("{}.tmp.{nanos}", file_name.to_string_lossy()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.flush()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 写盘分派（`write_live(app, provider)`）：按应用选择写盘实现，各自保持
/// 「只合并受控字段、非受控原地保留、写前备份、原子写」——
/// - claude：JSON 受控合并进 `~/.claude/settings.json`（本模块）。
/// - codex：TOML 受控合并进 `~/.codex/config.toml` + 受控写
///   `~/.codex/auth.json`（`live_codex` 模块）。
/// - gemini：`.env` 整块替换 + `settings.json` 受控合并
///   （`live_gemini` 模块，含 `selectedType` 认证标记）。
/// - grok：TOML 受控合并进 `~/.grok/config.toml`（`live_grok` 模块，
///   单文件无 auth；cc one 固定写 `[model."cc-one"]` profile + 设
///   `models.default`，用户其它 profile / mcp_servers 原样保留）。
pub fn write_live(app: App, provider: &Provider) -> AppResult<()> {
    match app {
        App::Claude => {
            let path = claude_settings_path()?;
            switch_live_settings(&path, &provider.settings_config)
        }
        App::Codex => {
            let config_path = crate::provider::live_codex::codex_config_path()?;
            let auth_path = crate::provider::live_codex::codex_auth_path()?;
            crate::provider::live_codex::switch_codex_live(
                &config_path,
                &auth_path,
                &provider.settings_config,
            )
        }
        App::Gemini => crate::provider::live_gemini::write_gemini_live(&provider.settings_config),
        App::Grok => {
            let config_path = crate::provider::live_grok::grok_config_path()?;
            crate::provider::live_grok::switch_grok_live(&config_path, &provider.settings_config)
        }
        // OpenCode 是附加模式，不走 write_live（单激活专属）——增删/切换走
        // set/remove_opencode_provider，由命令层按 is_additive_mode 分派。这里
        // 返回 Err 作防御：误调时明确报错，而非走单激活路径或 panic。
        App::OpenCode => Err(AppError::Config(
            "opencode is additive mode; use set/remove_opencode_provider, not write_live".into(),
        )),
    }
}

/// claude settings.json 的原子写（`atomic_write_file` 的既有公开面，调用方
/// 与测试沿用此名）。
pub fn write_live_settings(path: &Path, content: &str) -> AppResult<()> {
    atomic_write_file(path, content)
}

/// 切换写盘全流程（薄壳，按序调用）：读 live → 受控合并（含清洗）→ 备份 .bak →
/// 原子写。
pub fn switch_live_settings(path: &Path, settings_config: &str) -> AppResult<()> {
    let live = read_live_settings(path)?;
    let merged = merge_live_settings(&live, settings_config, LIVE_INTERNAL_KEYS)?;
    backup_live_settings(path)?;
    write_live_settings(path, &merged)?;
    Ok(())
}

/// 拒绝写盘前的未物化模板变量：settingsConfig 里残留 `${VAR}` 占位符（保存时
/// 前端已拦截，但导入的 JSON 或手改的元数据可能绕过）会以字面量形式写进用户
/// 的 settings.json——端点/密钥位置全是占位符，等于写一份废配置。宁可切换
/// 失败，也不静默写废。空串 → 无占位符（写盘按 `{}` 处理）。
pub fn validate_no_unfilled_template_vars(settings_config: &str) -> AppResult<()> {
    let Some(name) = find_unfilled_template_var(settings_config) else {
        return Ok(());
    };
    Err(AppError::Config(format!(
        "provider settingsConfig has an unfilled template variable: ${{{name}}}"
    )))
}

/// 第一个 `${VAR}` 占位符名；无 → `None`。与前端 `derive.ts` 的
/// `TEMPLATE_VAR_RE` 同一形状（`${` + 标识符 + `}`）。
fn find_unfilled_template_var(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > start && j < bytes.len() && bytes[j] == b'}' {
                return Some(String::from_utf8_lossy(&bytes[start..j]).into_owned());
            }
        }
        i += 1;
    }
    None
}

/// 解析 live 输入：空串/纯空白 → `{}`；非空但非法 JSON 或非对象 → `Err`。
/// `live_gemini` 复用同一条解析规则（现有 settings.json 缺失时视为 `{}`）。
pub(crate) fn parse_live_or_empty(live: &str) -> AppResult<serde_json::Value> {
    let trimmed = live.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    parse_object(trimmed, "live settings.json")
}

/// 解析目标输入：空串 → `{}`；非法 JSON 或非对象 → `Err`。
/// `live_gemini` 复用同一条解析规则（目标 settingsConfig 空串 = 空目标）。
pub(crate) fn parse_target_or_empty(target: &str) -> AppResult<serde_json::Value> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Object(Default::default()));
    }
    parse_object(trimmed, "provider settingsConfig")
}

/// 解析 JSON 文本为对象：非法 JSON 或非对象 → `Err`。供本模块的
/// `parse_live_or_empty` / `parse_target_or_empty` 与 `snippet` 模块共用
/// （片段校验与合并走同一条解析规则）。
pub(crate) fn parse_object(raw: &str, what: &str) -> AppResult<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(raw)
        .map_err(|e| AppError::Config(format!("{what} is not valid JSON: {e}")))?;
    if !v.is_object() {
        return Err(AppError::Config(format!("{what} is not a JSON object")));
    }
    Ok(v)
}

/// 解析 TOML 文本为可编辑文档：空串/纯空白 → 空文档；非法 TOML → `Err`。
/// codex / grok 的 TOML 受控合并共用（单一事实来源）——两个 live_* 模块都把
/// live / target 的 TOML 文本喂进来解析，不各自再抄一份。
pub(crate) fn parse_toml_or_empty(text: &str, what: &str) -> AppResult<DocumentMut> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>()
        .map_err(|e| AppError::Config(format!("{what} is not valid TOML: {e}")))
}

/// 解析供应商 settingsConfig JSON 文本为「剥过内部 meta 键的对象」：空串/
/// 纯空白 → `None`（登录态版）；非对象 → `Err`。剥 [`LIVE_INTERNAL_KEYS`]——这些
/// 键只供应用自己读，不是任何写盘文件（auth.json / config.toml）的合法字段。
/// codex / grok 两个 live_* 分支共用同一条「解析 + 清洗」前缀，各自只写后段
/// 的字段提取。
pub(crate) fn parse_and_strip_settings(
    settings_config: &str,
) -> AppResult<Option<serde_json::Value>> {
    let trimmed = settings_config.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut obj = parse_object(trimmed, "provider settingsConfig")?;
    if let Some(o) = obj.as_object_mut() {
        for key in LIVE_INTERNAL_KEYS {
            o.remove(*key);
        }
    }
    Ok(Some(obj))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 构造一份带非受控字段的 live 配置（模拟用户手动配了 hooks / MCP /
    /// permissions / model 的 settings.json）。
    fn live_with_uncontrolled(env: &str) -> String {
        format!(
            r#"{{
  "env": {env},
  "permissions": {{"allow": ["Bash"]}},
  "hooks": {{"PreToolUse": [{{"matcher": "Bash"}}]}},
  "mcpServers": {{"filesystem": {{"command": "npx"}}}},
  "enableAllProjectMcpServers": true,
  "model": "claude-sonnet-4-5",
  "extraKnownMarketplaces": ["marketplace.a"],
  "statusLine": {{"type": "command", "command": "echo hi"}}
}}"#
        )
    }

    /// 解析合并结果并返回对象引用。
    fn parsed(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn controlled_env_replaces_live_wholesale() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old", "KEEP_ME": "1"}"#);
        let target = r#"{
            "env": {"ANTHROPIC_BASE_URL": "https://new.dev", "ANTHROPIC_AUTH_TOKEN": "sk-new"},
            "includeCoAuthoredBy": false
        }"#;
        let out = parsed(&merge_live_settings(&live, target, LIVE_INTERNAL_KEYS).unwrap());
        // env 整块替换：live 的旧 env（含 KEEP_ME）全被覆盖。
        assert_eq!(
            out["env"],
            serde_json::json!({
                "ANTHROPIC_BASE_URL": "https://new.dev",
                "ANTHROPIC_AUTH_TOKEN": "sk-new"
            })
        );
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
    }

    #[test]
    fn uncontrolled_fields_kept_verbatim_from_live() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old"}"#);
        let target = r#"{"env": {"ANTHROPIC_MODEL": "new"}}"#;
        let out = parsed(&merge_live_settings(&live, target, LIVE_INTERNAL_KEYS).unwrap());
        // 非受控字段从 live 原样保留。
        assert_eq!(out["permissions"], serde_json::json!({"allow": ["Bash"]}));
        assert_eq!(
            out["hooks"],
            serde_json::json!({"PreToolUse": [{"matcher": "Bash"}]})
        );
        assert_eq!(
            out["mcpServers"],
            serde_json::json!({"filesystem": {"command": "npx"}})
        );
        assert_eq!(out["enableAllProjectMcpServers"], serde_json::json!(true));
        assert_eq!(out["model"], serde_json::json!("claude-sonnet-4-5"));
        assert_eq!(
            out["extraKnownMarketplaces"],
            serde_json::json!(["marketplace.a"])
        );
        assert_eq!(
            out["statusLine"],
            serde_json::json!({"type": "command", "command": "echo hi"})
        );
        // 受控字段 env 已替换。
        assert_eq!(out["env"], serde_json::json!({"ANTHROPIC_MODEL": "new"}));
    }

    #[test]
    fn target_uncontrolled_fields_are_ignored_not_written() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old"}"#);
        // 目标也带了 hooks / permissions / model——非受控，绝不能覆盖 live 的。
        let target = r#"{
            "env": {"ANTHROPIC_MODEL": "new"},
            "hooks": {"PostToolUse": [{"matcher": "*"}]},
            "permissions": {"deny": ["Bash"]},
            "model": "claude-opus-4-5"
        }"#;
        let out = parsed(&merge_live_settings(&live, target, LIVE_INTERNAL_KEYS).unwrap());
        assert_eq!(
            out["hooks"],
            serde_json::json!({"PreToolUse": [{"matcher": "Bash"}]}),
            "live 的 hooks 保留，目标的 hooks 被忽略"
        );
        assert_eq!(
            out["permissions"],
            serde_json::json!({"allow": ["Bash"]}),
            "live 的 permissions 保留"
        );
        assert_eq!(out["model"], serde_json::json!("claude-sonnet-4-5"));
    }

    #[test]
    fn missing_env_in_target_keeps_live_env() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_BASE_URL": "https://live.dev"}"#);
        // 目标没有 env（只有受控开关）——live 的 env 原样保留。
        let target = r#"{"includeCoAuthoredBy": true}"#;
        let out = parsed(&merge_live_settings(&live, target, LIVE_INTERNAL_KEYS).unwrap());
        assert_eq!(
            out["env"],
            serde_json::json!({"ANTHROPIC_BASE_URL": "https://live.dev"}),
            "目标缺失 env 时不得清空 live 的 env"
        );
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(true));
    }

    #[test]
    fn explicit_empty_env_replaces_live_env() {
        let live = live_with_uncontrolled(r#"{"ANTHROPIC_BASE_URL": "https://live.dev"}"#);
        // 目标显式写了空 env =「该供应商不想要任何 env」→ 整块替换成空。
        let target = r#"{"env": {}}"#;
        let out = parsed(&merge_live_settings(&live, target, LIVE_INTERNAL_KEYS).unwrap());
        assert_eq!(out["env"], serde_json::json!({}));
        // 非受控字段仍保留。
        assert_eq!(out["permissions"], serde_json::json!({"allow": ["Bash"]}));
    }

    #[test]
    fn empty_live_merges_to_sanitized_target() {
        let out = parsed(
            &merge_live_settings(
                "",
                r#"{"env": {"ANTHROPIC_MODEL": "m"}, "includeCoAuthoredBy": false}"#,
                LIVE_INTERNAL_KEYS,
            )
            .unwrap(),
        );
        assert_eq!(out["env"], serde_json::json!({"ANTHROPIC_MODEL": "m"}));
        assert_eq!(out["includeCoAuthoredBy"], serde_json::json!(false));
    }

    #[test]
    fn invalid_live_json_is_an_error() {
        let r = merge_live_settings("{not json", r#"{"env":{}}"#, LIVE_INTERNAL_KEYS);
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "live 非法 JSON 必须失败"
        );
    }

    #[test]
    fn non_object_live_is_an_error() {
        let r = merge_live_settings(r#"[1,2,3]"#, r#"{"env":{}}"#, LIVE_INTERNAL_KEYS);
        assert!(matches!(r, Err(AppError::Config(_))), "live 非对象必须失败");
    }

    #[test]
    fn invalid_target_json_is_an_error() {
        let r = merge_live_settings("{}", "{nope", LIVE_INTERNAL_KEYS);
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "目标非法 JSON 必须失败"
        );
    }

    #[test]
    fn non_object_target_is_an_error() {
        let r = merge_live_settings("{}", r#""just a string""#, LIVE_INTERNAL_KEYS);
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn non_object_target_env_is_an_error() {
        // 目标 env 非对象（手写/导入的坏配置）——若放行会被整块写进用户的
        // settings.json，必须报错阻止写盘。
        for bad in [r#"{"env": "garbage"}"#, r#"{"env": ["A=1"]}"#] {
            let r = merge_live_settings("{}", bad, LIVE_INTERNAL_KEYS);
            assert!(
                matches!(r, Err(AppError::Config(_))),
                "目标 env 非对象必须失败: {bad}"
            );
        }
    }

    #[test]
    fn sanitize_strips_internal_keys_from_target_and_live() {
        // 目标带着应用内部 meta 字段（cc-switch 遗留的写法）——必须被剥掉。
        let target = r#"{
            "api_format": "anthropic",
            "apiFormat": "anthropic",
            "openrouter_compat_mode": true,
            "openrouterCompatMode": true,
            "env": {"ANTHROPIC_MODEL": "m"}
        }"#;
        let out = merge_live_settings("{}", target, LIVE_INTERNAL_KEYS).unwrap();
        let v = parsed(&out);
        assert!(v.get("api_format").is_none(), "api_format 必须被剥");
        assert!(v.get("apiFormat").is_none(), "apiFormat 必须被剥");
        assert!(v.get("openrouter_compat_mode").is_none());
        assert!(v.get("openrouterCompatMode").is_none());
        assert_eq!(v["env"], serde_json::json!({"ANTHROPIC_MODEL": "m"}));

        // live 里残留的内部键同样被清掉（写出去的 live 永远不含内部字段）。
        let live = r#"{"api_format": "anthropic", "permissions": {"allow": ["Bash"]}}"#;
        let out2 = merge_live_settings(live, r#"{"env":{}}"#, LIVE_INTERNAL_KEYS).unwrap();
        let v2 = parsed(&out2);
        assert!(v2.get("api_format").is_none());
        assert_eq!(v2["permissions"], serde_json::json!({"allow": ["Bash"]}));
    }

    #[test]
    fn backup_creates_bak_when_live_exists_and_skips_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        // live 不存在 → 跳过备份。
        backup_live_settings(&path).unwrap();
        assert!(!path.with_extension("json.bak").exists());

        fs::write(&path, r#"{"env":{}}"#).unwrap();
        backup_live_settings(&path).unwrap();
        let bak = path.with_extension("json.bak");
        assert!(bak.exists(), "live 存在时必须生成 .bak");
        assert_eq!(fs::read_to_string(&bak).unwrap(), r#"{"env":{}}"#);

        // 单份覆盖：再次备份，旧 .bak 被新内容覆盖，不会堆积多份。
        fs::write(&path, r#"{"env":{"A":"2"}}"#).unwrap();
        backup_live_settings(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&bak).unwrap(),
            r#"{"env":{"A":"2"}}"#,
            ".bak 单份覆盖，不追加不堆积"
        );
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_and_replaces_target() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(&path, "old").unwrap();
        write_live_settings(&path, r#"{"env":{"A":"1"}}"#).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"env":{"A":"1"}}"#);
        // 临时文件已改名，目录里没有残留 .tmp.*。
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains("settings.json.tmp.")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "原子写后不得残留临时文件: {leftovers:?}"
        );
    }

    #[test]
    fn switch_live_settings_runs_full_flow_and_preserves_uncontrolled() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        fs::write(
            &path,
            live_with_uncontrolled(r#"{"ANTHROPIC_MODEL": "old"}"#),
        )
        .unwrap();

        switch_live_settings(&path, r#"{"env":{"ANTHROPIC_MODEL":"new"}}"#).unwrap();

        let written = fs::read_to_string(&path).unwrap();
        let v = parsed(&written);
        assert_eq!(v["env"], serde_json::json!({"ANTHROPIC_MODEL": "new"}));
        assert_eq!(
            v["permissions"],
            serde_json::json!({"allow": ["Bash"]}),
            "非受控字段经完整流程后仍保留"
        );
        // 备份内容 = 写盘前的 live。
        let bak = fs::read_to_string(path.with_extension("json.bak")).unwrap();
        let bak_v = parsed(&bak);
        assert_eq!(
            bak_v["env"],
            serde_json::json!({"ANTHROPIC_MODEL": "old"}),
            ".bak 是写盘前的 live 快照"
        );
    }

    #[test]
    fn switch_live_settings_when_live_missing_creates_file_no_bak() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("settings.json");
        switch_live_settings(&path, r#"{"env":{"ANTHROPIC_MODEL":"m"}}"#).unwrap();
        assert!(path.exists());
        let v = parsed(&fs::read_to_string(&path).unwrap());
        assert_eq!(v["env"], serde_json::json!({"ANTHROPIC_MODEL": "m"}));
        assert!(
            !path.with_extension("json.bak").exists(),
            "live 原本不存在 → 无备份"
        );
    }

    #[test]
    fn claude_settings_path_points_at_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            claude_settings_path().unwrap(),
            home.join(".claude").join("settings.json")
        );
    }

    #[test]
    fn validate_no_unfilled_template_vars_rejects_placeholders() {
        // 未物化的占位符 → 拒绝写盘。
        let bad = r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.${AWS_REGION}.amazonaws.com"}}"#;
        let err = validate_no_unfilled_template_vars(bad).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("AWS_REGION"), "报错要指出是哪个变量: {msg}");
        // 物化后 → 通过；空串 → 通过。
        assert!(validate_no_unfilled_template_vars(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://bedrock-runtime.us-east-1.amazonaws.com"}}"#
        )
        .is_ok());
        assert!(validate_no_unfilled_template_vars("  ").is_ok());
    }
}
