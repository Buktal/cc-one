//! Gemini 写盘：`~/.gemini/.env`（键值）+ `~/.gemini/settings.json`。
//!
//! 写盘语义（与 claude 侧同一套「只合并受控字段」规则的 gemini 形态）：
//! - **env 整块替换**：`.env` 是受控单位——切换供应商时整块重写（含空 env
//!   = 登录态版），与 claude 的 env 整块替换同一语义；写前备份 `.env.bak`
//!   （单份覆盖），原子写（临时文件 + 改名）。
//! - **settings.json 受控合并**：只写受控字段——供应商 `config` 对象里声明
//!   的顶层字段，以及 `security.auth.selectedType` 认证标记（env 含
//!   `GEMINI_API_KEY` → `"gemini-api-key"`，否则 → `"oauth"`，两分支即
//!   API Key 版 / 登录态版）。顶层身份键（见 [`GEMINI_CONTROLLED_FIELDS`]）
//!   按「新供应商赢：携带 → 替换，缺失 → 撤除」合并（防旧供应商残留）；
//!   `mcpServers` 等其余字段从现有文件原样保留，绝不整文件覆盖。
//! - **登录态版**（env 无 `GEMINI_API_KEY`）：env 写成空 + `selectedType:
//!   "oauth"`，不破坏用户既有 Google 登录态。
//! - 清洗：写盘前剥掉 settingsConfig 顶层内部 meta 字段（沿用
//!   [`live::LIVE_INTERNAL_KEYS`] 语义）。
//! - 两文件同备份同原子写：`.env.bak` + `settings.json.bak`（单份覆盖），各
//!   自临时文件 + 改名；合并与校验全部在任何写盘之前完成，两文件内容无变化
//!   则整体无操作（不备份、不写盘、不碰 mtime，与其它四个 app 同一事务语义，
//!   原语在 [`live`]）；settings 写失败回滚 .env——任何失败路径不产生半截
//!   状态（与 codex 侧同一容错级别）。
//!
//! 纯函数（最高价值测试接缝，不碰文件系统）：`parse_gemini_settings`
//! （settingsConfig 文本 → `{env, config}`，含校验与清洗）、
//! `serialize_env_file`（键值对 → `.env` 文本，按键排序字节稳定）、
//! `gemini_selected_type`（env → 认证标记）、`merge_gemini_settings_json`
//! （现有 settings.json 文本 + 目标 → 合并文本）。文件 IO（读/备份/原子写）
//! 是薄壳：备份与原子写分别见本模块与 [`live::atomic_write_file`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::provider::live::{
    atomic_write_file, parse_live_or_empty, parse_target_or_empty, read_live_settings,
    strip_internal_keys,
};

/// `settings.json` 里 `security.auth.selectedType` 的 API Key 标记对应的
/// env 键：env 含它 → API Key 版；不含 → 登录态版。
pub const GEMINI_API_KEY_ENV: &str = "GEMINI_API_KEY";
/// `selectedType` 取值：API Key 版（env 含 `GEMINI_API_KEY`）。
pub const SELECTED_TYPE_API_KEY: &str = "gemini-api-key";
/// `selectedType` 取值：登录态版（env 无 `GEMINI_API_KEY`，保留 Google 登录）。
pub const SELECTED_TYPE_OAUTH: &str = "oauth";

/// Gemini settings.json 顶层受控身份键（与 codex 的 `CODEX_CONTROLLED_FIELDS`
/// 同一语义）：供应商 `config` 携带 → 替换；缺失 → 从 live 撤除（受控轴
/// 「新供应商赢」——旧供应商的 `model` 残留会让切换静默失效）。env 侧的模型
/// 选择（`GEMINI_MODEL`）随 `.env` 整块替换，不受此清单管；`mcpServers` 等
/// 用户手动键是非受控字段，永不被清单撤除。
pub const GEMINI_CONTROLLED_FIELDS: &[&str] = &["model"];

/// Gemini 配置目录：`~/.gemini`。
pub fn gemini_dir() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".gemini"))
}

/// `~/.gemini/.env` 路径。
pub fn gemini_env_path() -> AppResult<PathBuf> {
    Ok(gemini_dir()?.join(".env"))
}

/// `~/.gemini/settings.json` 路径。
pub fn gemini_settings_path() -> AppResult<PathBuf> {
    Ok(gemini_dir()?.join("settings.json"))
}

/// 解析后的 Gemini 目标配置：`env`（整块写 `.env`）与 `config`（写
/// settings.json 的受控字段；缺失或 `null` → `None`，表示不合并任何字段、
/// 现有文件原样保留）。
#[derive(Debug, Clone, PartialEq)]
pub struct GeminiSettings {
    pub env: HashMap<String, String>,
    pub config: Option<serde_json::Map<String, serde_json::Value>>,
}

/// 解析 + 校验 + 清洗（纯函数）：settingsConfig 文本 → [`GeminiSettings`]。
///
/// 边界：
/// - settingsConfig 必须是 JSON 对象，否则 `Err`（坏配置不能进用户配置文件）。
/// - `env` 必须缺失或是对象，且值必须是字符串（`.env` 只能表达键值）；
///   非对象或非字符串值 → `Err`（静默丢键会让认证悄悄坏掉，宁可失败）。
/// - `config` 必须缺失、是对象或 `null`（`null` 与缺失同义 → `None`）；
///   其余类型 → `Err`。
/// - 顶层剥掉内部 meta 字段（[`LIVE_INTERNAL_KEYS`]，与 claude 侧清洗同一
///   语义——这些键只供应用内部读，不落 live 文件）。
pub fn parse_gemini_settings(raw: &str) -> AppResult<GeminiSettings> {
    let mut value = parse_target_or_empty(raw)?;
    let obj = value
        .as_object_mut()
        .expect("parse_target_or_empty yields object");
    strip_internal_keys(obj);

    let env = match obj.remove("env") {
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

    let config = match obj.remove("config") {
        None | Some(serde_json::Value::Null) => None,
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

/// 键值对 → `.env` 文本（纯函数）：按键排序保证字节稳定，每行 `KEY=VALUE`，
/// 行间换行、结尾无换行（与 cc-switch 同款输出）。空 map → 空串。
pub fn serialize_env_file(env: &HashMap<String, String>) -> String {
    let mut keys: Vec<&String> = env.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|k| format!("{k}={}", env[k]))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 认证模式标记（纯函数）：env 含 `GEMINI_API_KEY` → API Key 版
/// （`"gemini-api-key"`）；否则 → 登录态版（`"oauth"`，保留用户既有 Google
/// 登录）。与写盘契约的两分支一致。
pub fn gemini_selected_type(env: &HashMap<String, String>) -> &'static str {
    if env.contains_key(GEMINI_API_KEY_ENV) {
        SELECTED_TYPE_API_KEY
    } else {
        SELECTED_TYPE_OAUTH
    }
}

/// 受控合并纯函数（测试接缝）：现有 `settings.json` 文本 + 目标 →
/// 合并后的 settings.json 文本，不碰文件系统。
///
/// 语义：
/// - 现有文本为空串/纯空白 → 视为 `{}`（文件缺失时由 `{}` 新建）；非空但
///   非法 JSON 或非对象 → `Err`（解析不了就没法保留用户手动配置，宁可失败）。
/// - 目标 `config` 的顶层字段合并进结果（供应商显式配置优先）。
/// - 顶层身份键（[`GEMINI_CONTROLLED_FIELDS`]）按「新供应商赢」：config 携带
///   → 替换；config 缺失或不含该键 → 从现有文件撤除（防旧供应商残留）。
/// - `security.auth.selectedType` 恒按 env 推导写受控标记（两分支见
///   [`gemini_selected_type`]）。现有 `security` / `auth` 若存在但非对象 →
///   `Err`（标记写不进去，宁可失败）。
/// - 其余字段（`mcpServers` 等）一律从现有文件原样保留，绝不整文件覆盖。
pub fn merge_gemini_settings_json(existing: &str, target: &GeminiSettings) -> AppResult<String> {
    let mut merged = parse_live_or_empty(existing)?;
    let merged_obj = merged
        .as_object_mut()
        .expect("parse_live_or_empty yields object");

    if let Some(config) = &target.config {
        for (key, value) in config {
            merged_obj.insert(key.clone(), value.clone());
        }
    }
    // 顶层身份键撤除：config 不携带即撤（ADR-0010 受控轴「新供应商赢」）。
    for key in GEMINI_CONTROLLED_FIELDS {
        let carried = target.config.as_ref().is_some_and(|c| c.contains_key(*key));
        if !carried {
            merged_obj.remove(*key);
        }
    }

    let security = merged_obj
        .entry("security")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let security_obj = security.as_object_mut().ok_or_else(|| {
        AppError::Config("existing gemini settings.json security is not a JSON object".into())
    })?;
    let auth = security_obj
        .entry("auth")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let auth_obj = auth.as_object_mut().ok_or_else(|| {
        AppError::Config("existing gemini settings.json security.auth is not a JSON object".into())
    })?;
    auth_obj.insert(
        "selectedType".to_string(),
        serde_json::Value::String(gemini_selected_type(&target.env).to_string()),
    );

    Ok(serde_json::to_string_pretty(&merged)?)
}

/// `.env` 的备份路径：`<dir>/.env` → `<dir>/.env.bak`。显式拼文件名，不依赖
/// `with_extension` 对无扩展名点文件的行为。
pub fn env_backup_path(env_path: &Path) -> PathBuf {
    let file_name = env_path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".env".to_string());
    env_path
        .parent()
        .unwrap_or(Path::new(""))
        .join(format!("{file_name}.bak"))
}

/// 备份 `.env` 为 `.env.bak`（单份覆盖）；`.env` 不存在 → 跳过（没有可备份
/// 的内容）。与 claude 侧 settings.json.bak 同一备份语义。
pub fn backup_env_file(env_path: &Path) -> AppResult<()> {
    if !env_path.exists() {
        return Ok(());
    }
    std::fs::copy(env_path, env_backup_path(env_path))?;
    Ok(())
}

/// Gemini 写盘全流程（薄壳，按序调用，路径注入便于测试）：解析+清洗 → 读
/// 两文件现状 → 受控合并（三步全部在任何写盘/备份之前完成——用户手改坏的
/// settings.json 在这里失败，两文件同旧）→ 两文件内容无变化则整体无操作 →
/// 备份并原子写 `.env`（整块替换）→ 备份并原子写 settings.json；settings
/// 一步失败回滚 `.env`——任何失败路径不产生半截状态（与 codex 侧同一容错
/// 语义，回滚原语共用 `live::rollback_side_file`）。
pub fn write_gemini_live_at(
    env_path: &Path,
    settings_path: &Path,
    settings_config: &str,
) -> AppResult<()> {
    let target = parse_gemini_settings(settings_config)?;
    let existing_env: Option<String> = match std::fs::read_to_string(env_path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let existing = read_live_settings(settings_path)?;
    let merged = merge_gemini_settings_json(&existing, &target)?;
    let env_text = serialize_env_file(&target.env);
    // `.env` 的无变化判定要求「文件存在且内容不变」：文件缺失时即便目标为空
    // 也要写出（建空 .env 本身是登录态版语义的一部分）；settings.json 用通用
    // trim_end 比较（`.env` 的备份走 dotfile 专属路径 `backup_env_file`，不经
    // `commit_live_file`）。
    let env_unchanged = existing_env
        .as_deref()
        .is_some_and(|old| crate::provider::live::content_unchanged(old, &env_text));
    let settings_unchanged = crate::provider::live::content_unchanged(&existing, &merged);
    if env_unchanged && settings_unchanged {
        return Ok(());
    }

    let env_committed = if env_unchanged {
        false
    } else {
        backup_env_file(env_path)?;
        atomic_write_file(env_path, &env_text)?;
        true
    };
    if let Err(e) =
        crate::provider::live::commit_live_file(settings_path, &merged, settings_unchanged)
    {
        if env_committed {
            crate::provider::live::rollback_side_file(
                env_path,
                &existing_env,
                "gemini settings.json write failed and .env rollback",
            );
        }
        return Err(e);
    }
    Ok(())
}

/// 真实路径入口：写 `~/.gemini/.env` + `~/.gemini/settings.json`。
pub fn write_gemini_live(settings_config: &str) -> AppResult<()> {
    write_gemini_live_at(
        &gemini_env_path()?,
        &gemini_settings_path()?,
        settings_config,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 解析合并/写盘结果 JSON 并返回对象。
    fn parsed(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    /// 一份带 mcpServers / model 等非受控字段的现有 settings.json（模拟用户
    /// 手动配置或 Gemini CLI 自己写的文件）。
    fn existing_settings(selected_type: &str) -> String {
        format!(
            r#"{{
  "model": "gemini-2.5-pro",
  "mcpServers": {{
    "filesystem": {{"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem"]}}
  }},
  "security": {{
    "auth": {{
      "selectedType": "{selected_type}"
    }}
  }}
}}"#
        )
    }

    fn api_key_target() -> GeminiSettings {
        let mut env = HashMap::new();
        env.insert("GEMINI_API_KEY".to_string(), "sk-gemini-123".to_string());
        env.insert(
            "GOOGLE_GEMINI_BASE_URL".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
        );
        env.insert("GEMINI_MODEL".to_string(), "gemini-3-flash".to_string());
        GeminiSettings {
            env,
            config: Some(
                parsed(r#"{"model": "gemini-3-flash"}"#)
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        }
    }

    fn oauth_target() -> GeminiSettings {
        GeminiSettings {
            env: HashMap::new(),
            config: None,
        }
    }

    // ---- parse ----

    #[test]
    fn parse_extracts_env_and_config() {
        let s = parse_gemini_settings(
            r#"{"env":{"GEMINI_API_KEY":"sk-x","GEMINI_MODEL":"m"},"config":{"model":"m"}}"#,
        )
        .unwrap();
        assert_eq!(
            s.env.get("GEMINI_API_KEY").map(String::as_str),
            Some("sk-x")
        );
        assert_eq!(s.env.get("GEMINI_MODEL").map(String::as_str), Some("m"));
        let config = s.config.expect("config parsed");
        assert_eq!(config.get("model"), Some(&serde_json::json!("m")));
    }

    #[test]
    fn parse_accepts_missing_env_and_null_config() {
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
    fn parse_strips_internal_meta_keys_from_top_level() {
        let s = parse_gemini_settings(
            r#"{"api_format":"gemini","apiFormat":"gemini","openrouter_compat_mode":true,"env":{"GEMINI_API_KEY":"k"}}"#,
        )
        .unwrap();
        assert_eq!(s.env.get("GEMINI_API_KEY").map(String::as_str), Some("k"));
        // 内部键不进 config、不进 env——它们只供应用内部读。
        assert!(s.config.is_none());
    }

    #[test]
    fn parse_rejects_invalid_json_and_non_object() {
        for bad in ["{oops", r#"[1,2,3]"#, r#""str""#] {
            assert!(
                parse_gemini_settings(bad).is_err(),
                "非法/非对象 settingsConfig 必须失败: {bad}"
            );
        }
    }

    #[test]
    fn parse_rejects_non_object_env_and_non_string_values() {
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
    fn parse_rejects_config_that_is_neither_object_nor_null() {
        for bad in [r#"{"config":123}"#, r#"{"config":"x"}"#, r#"{"config":[]}"#] {
            assert!(
                parse_gemini_settings(bad).is_err(),
                "config 非对象非 null 必须失败: {bad}"
            );
        }
    }

    // ---- serialize / selectedType ----

    #[test]
    fn serialize_env_file_sorts_keys_and_is_stable() {
        let mut env = HashMap::new();
        env.insert("GEMINI_MODEL".to_string(), "gemini-3-flash".to_string());
        env.insert("GEMINI_API_KEY".to_string(), "sk-x".to_string());
        env.insert(
            "GOOGLE_GEMINI_BASE_URL".to_string(),
            "https://generativelanguage.googleapis.com".to_string(),
        );
        let text = serialize_env_file(&env);
        assert_eq!(
            text,
            "GEMINI_API_KEY=sk-x\nGEMINI_MODEL=gemini-3-flash\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com"
        );
        assert_eq!(text, serialize_env_file(&env), "字节稳定");
        assert_eq!(serialize_env_file(&HashMap::new()), "");
    }

    #[test]
    fn selected_type_follows_gemini_api_key_presence() {
        let mut keyed = HashMap::new();
        keyed.insert("GEMINI_API_KEY".to_string(), "sk-x".to_string());
        assert_eq!(gemini_selected_type(&keyed), SELECTED_TYPE_API_KEY);
        assert_eq!(gemini_selected_type(&HashMap::new()), SELECTED_TYPE_OAUTH);
        // 只有 base URL 没有 key → 登录态（key 才是 API Key 版的判据）。
        let mut base_only = HashMap::new();
        base_only.insert(
            "GOOGLE_GEMINI_BASE_URL".to_string(),
            "https://x.dev".to_string(),
        );
        assert_eq!(gemini_selected_type(&base_only), SELECTED_TYPE_OAUTH);
    }

    // ---- merge ----

    #[test]
    fn merge_preserves_uncontrolled_and_merges_controlled_fields() {
        let target = api_key_target();
        let out =
            parsed(&merge_gemini_settings_json(&existing_settings("oauth"), &target).unwrap());
        // 受控：config 字段合并（供应商显式配置优先）。
        assert_eq!(out["model"], serde_json::json!("gemini-3-flash"));
        // 非受控：mcpServers 从现有文件原样保留。
        assert_eq!(
            out["mcpServers"]["filesystem"]["command"],
            serde_json::json!("npx")
        );
        // 受控标记：API Key 版。
        assert_eq!(
            out["security"]["auth"]["selectedType"],
            serde_json::json!("gemini-api-key")
        );
        // security 的其他字段（如有）原样保留。
        assert!(out["security"]["auth"].get("selectedType").is_some());
    }

    #[test]
    fn merge_oauth_branch_writes_oauth_marker_and_preserves_existing() {
        let target = oauth_target();
        let out = parsed(
            &merge_gemini_settings_json(&existing_settings("gemini-api-key"), &target).unwrap(),
        );
        // 登录态版：config None → 顶层身份键撤除（model 不残留），mcpServers
        // 等非受控字段原样保留，只把标记改为 oauth。
        assert!(
            out.get("model").is_none(),
            "config 缺失的身份键必须撤除，不得残留旧供应商的 model"
        );
        assert_eq!(
            out["mcpServers"]["filesystem"]["command"],
            serde_json::json!("npx")
        );
        assert_eq!(
            out["security"]["auth"]["selectedType"],
            serde_json::json!("oauth")
        );
    }

    /// 身份键撤除语义：config 携带 → 替换；config 缺失或不携带该键 → 撤除；
    /// 非受控键不受清单影响。
    #[test]
    fn merge_withdraws_identity_keys_not_carried_by_target() {
        // 目标 config 只带 model → 替换为供应商值。
        let mut cfg = serde_json::Map::new();
        cfg.insert(
            "model".to_string(),
            serde_json::Value::String("gemini-3-flash".to_string()),
        );
        let target = GeminiSettings {
            env: HashMap::new(),
            config: Some(cfg),
        };
        let out =
            parsed(&merge_gemini_settings_json(&existing_settings("oauth"), &target).unwrap());
        assert_eq!(out["model"], serde_json::json!("gemini-3-flash"));

        // 目标 config 带其它键但不带 model → model 撤除；mcpServers 保留。
        let mut cfg2 = serde_json::Map::new();
        cfg2.insert(
            "selectedTheme".to_string(),
            serde_json::Value::String("auto".to_string()),
        );
        let target2 = GeminiSettings {
            env: HashMap::new(),
            config: Some(cfg2),
        };
        let out2 =
            parsed(&merge_gemini_settings_json(&existing_settings("oauth"), &target2).unwrap());
        assert!(out2.get("model").is_none(), "未携带的身份键撤除");
        assert_eq!(out2["selectedTheme"], serde_json::json!("auto"));
        assert_eq!(
            out2["mcpServers"]["filesystem"]["command"],
            serde_json::json!("npx"),
            "非受控键不被清单撤除"
        );
    }

    #[test]
    fn merge_with_missing_file_creates_marker_only() {
        // 文件不存在（空输入）→ 新建：只写受控标记。
        let target = oauth_target();
        let out = parsed(&merge_gemini_settings_json("", &target).unwrap());
        assert_eq!(
            out,
            serde_json::json!({"security": {"auth": {"selectedType": "oauth"}}})
        );
        let keyed = api_key_target();
        let out2 = parsed(&merge_gemini_settings_json("   \n", &keyed).unwrap());
        assert_eq!(
            out2["security"]["auth"]["selectedType"],
            serde_json::json!("gemini-api-key")
        );
    }

    #[test]
    fn merge_rejects_invalid_existing_and_non_object_security() {
        // 现有文件解析不了 → 无法保留用户手动配置，宁可失败。
        assert!(merge_gemini_settings_json("{oops", &oauth_target()).is_err());
        assert!(merge_gemini_settings_json(r#"[1]"#, &oauth_target()).is_err());
        // security / auth 存在但非对象 → 标记写不进去，宁可失败。
        assert!(merge_gemini_settings_json(r#"{"security":"x"}"#, &oauth_target()).is_err());
        assert!(
            merge_gemini_settings_json(r#"{"security":{"auth":[]}}"#, &oauth_target()).is_err()
        );
        assert!(merge_gemini_settings_json(r#"{"security":null}"#, &oauth_target()).is_err());
    }

    // ---- backup / atomic write / full flow ----

    #[test]
    fn env_backup_path_is_dot_env_bak() {
        let p = Path::new("/home/u/.gemini/.env");
        assert_eq!(
            env_backup_path(p),
            PathBuf::from("/home/u/.gemini/.env.bak")
        );
    }

    #[test]
    fn backup_env_file_skips_missing_and_overwrites_single_copy() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        backup_env_file(&env_path).unwrap();
        assert!(!env_backup_path(&env_path).exists(), "无 .env → 无备份");

        fs::write(&env_path, "OLD=1").unwrap();
        backup_env_file(&env_path).unwrap();
        let bak = env_backup_path(&env_path);
        assert_eq!(fs::read_to_string(&bak).unwrap(), "OLD=1");
        // 单份覆盖：再次备份，旧 .bak 被新内容覆盖，不堆积。
        fs::write(&env_path, "NEW=2").unwrap();
        backup_env_file(&env_path).unwrap();
        assert_eq!(fs::read_to_string(&bak).unwrap(), "NEW=2");
    }

    /// 跑一次完整写盘：预置旧 .env + 现有 settings.json，写 API Key 版目标，
    /// 断言两文件与备份的最终内容。
    #[test]
    fn write_gemini_live_at_api_key_mode_full_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        fs::write(&env_path, "GEMINI_MODEL=old-model\nKEEP_ME=1\n").unwrap();
        fs::write(&settings_path, existing_settings("oauth")).unwrap();

        write_gemini_live_at(
            &env_path,
            &settings_path,
            r#"{"env":{"GEMINI_API_KEY":"sk-new","GOOGLE_GEMINI_BASE_URL":"https://gen.dev","GEMINI_MODEL":"gemini-3-flash"},"config":{"model":"gemini-3-flash"}}"#,
        )
        .unwrap();

        // env 整块替换：旧键全清，只含目标 env。
        let env_text = fs::read_to_string(&env_path).unwrap();
        assert_eq!(
            env_text,
            "GEMINI_API_KEY=sk-new\nGEMINI_MODEL=gemini-3-flash\nGOOGLE_GEMINI_BASE_URL=https://gen.dev"
        );
        // 备份 = 写盘前的 .env。
        assert_eq!(
            fs::read_to_string(env_backup_path(&env_path)).unwrap(),
            "GEMINI_MODEL=old-model\nKEEP_ME=1\n"
        );
        // settings.json：mcpServers 保留 + config 合并 + API Key 标记。
        let settings = parsed(&fs::read_to_string(&settings_path).unwrap());
        assert_eq!(
            settings["mcpServers"]["filesystem"]["command"],
            serde_json::json!("npx")
        );
        assert_eq!(settings["model"], serde_json::json!("gemini-3-flash"));
        assert_eq!(
            settings["security"]["auth"]["selectedType"],
            serde_json::json!("gemini-api-key")
        );
        // 原子写：无残留临时文件。
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "原子写后不得残留临时文件: {leftovers:?}"
        );
    }

    /// 登录态版完整写盘：env 整块替换为空（保留用户既有 Google 登录），
    /// settings.json 只改标记为 oauth。
    #[test]
    fn write_gemini_live_at_oauth_mode_full_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        fs::write(&env_path, "GEMINI_API_KEY=sk-old\n").unwrap();
        fs::write(&settings_path, existing_settings("gemini-api-key")).unwrap();

        write_gemini_live_at(&env_path, &settings_path, r#"{"env":{}}"#).unwrap();

        assert_eq!(
            fs::read_to_string(&env_path).unwrap(),
            "",
            "登录态版 env 为空"
        );
        assert_eq!(
            fs::read_to_string(env_backup_path(&env_path)).unwrap(),
            "GEMINI_API_KEY=sk-old\n",
            "备份是写盘前的旧 env"
        );
        let settings = parsed(&fs::read_to_string(&settings_path).unwrap());
        assert_eq!(
            settings["security"]["auth"]["selectedType"],
            serde_json::json!("oauth")
        );
        assert!(
            settings.get("model").is_none(),
            "登录态版撤除旧供应商的顶层身份键 model"
        );
        assert_eq!(
            settings["mcpServers"]["filesystem"]["command"],
            serde_json::json!("npx"),
            "登录态版同样保留 mcpServers"
        );
        // 备份对称：settings.json 与 .env 同等备份。
        assert_eq!(
            fs::read_to_string(tmp.path().join("settings.json.bak")).unwrap(),
            existing_settings("gemini-api-key"),
            ".bak 是写盘前的 settings.json"
        );
    }

    /// 文件全缺失的边界：env 与 settings.json 都新建，env 无备份。
    #[test]
    fn write_gemini_live_at_creates_files_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        write_gemini_live_at(&env_path, &settings_path, r#"{"env":{}}"#).unwrap();
        assert!(env_path.exists());
        assert_eq!(fs::read_to_string(&env_path).unwrap(), "");
        assert!(!env_backup_path(&env_path).exists(), "原本不存在 → 无备份");
        let settings = parsed(&fs::read_to_string(&settings_path).unwrap());
        assert_eq!(
            settings,
            serde_json::json!({"security": {"auth": {"selectedType": "oauth"}}})
        );
    }

    /// 重复切换同一供应商 → 整体无操作（两文件都不重写、不新建 .bak、mtime
    /// 不动）。与其它四个 app 的既有事务语义对齐（此前 gemini 缺失）。
    #[test]
    fn write_gemini_live_at_no_change_is_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        // 先种旧内容：第一次切换是真实写盘（产生 .bak），第二次同目标无操作。
        fs::write(&env_path, "OLD=1\n").unwrap();
        fs::write(&settings_path, r#"{"mcpServers":{}}"#).unwrap();
        let target = r#"{"env":{"GEMINI_API_KEY":"sk-x"}}"#;
        write_gemini_live_at(&env_path, &settings_path, target).unwrap();
        let env_before = fs::read_to_string(&env_path).unwrap();
        let settings_before = fs::read_to_string(&settings_path).unwrap();
        // 第一次写盘产生的 .bak 删掉，无变化切换不得重建它们。
        fs::remove_file(env_backup_path(&env_path)).unwrap();
        fs::remove_file(tmp.path().join("settings.json.bak")).unwrap();
        let env_mtime = fs::metadata(&env_path).unwrap().modified().unwrap();
        let settings_mtime = fs::metadata(&settings_path).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        write_gemini_live_at(&env_path, &settings_path, target).unwrap();
        assert_eq!(fs::read_to_string(&env_path).unwrap(), env_before);
        assert_eq!(fs::read_to_string(&settings_path).unwrap(), settings_before);
        assert!(
            !env_backup_path(&env_path).exists(),
            ".env 无变化不得触发备份"
        );
        assert!(
            !tmp.path().join("settings.json.bak").exists(),
            "settings.json 无变化不得触发备份"
        );
        assert_eq!(
            fs::metadata(&env_path).unwrap().modified().unwrap(),
            env_mtime,
            "无变化不得重写 .env（mtime 不得变化）"
        );
        assert_eq!(
            fs::metadata(&settings_path).unwrap().modified().unwrap(),
            settings_mtime,
            "无变化不得重写 settings.json（mtime 不得变化）"
        );
    }

    /// 坏配置必须在任何写盘动作之前失败：env / settings.json / .bak 全都不动。
    #[test]
    fn write_gemini_live_at_rejects_bad_config_before_writing() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        fs::write(&env_path, "KEEP=1\n").unwrap();
        fs::write(&settings_path, r#"{"mcpServers":{}}"#).unwrap();

        assert!(write_gemini_live_at(&env_path, &settings_path, "{oops").is_err());
        assert!(write_gemini_live_at(&env_path, &settings_path, r#"{"env":123}"#).is_err());
        assert_eq!(fs::read_to_string(&env_path).unwrap(), "KEEP=1\n");
        assert_eq!(
            fs::read_to_string(&settings_path).unwrap(),
            r#"{"mcpServers":{}}"#
        );
        assert!(!env_backup_path(&env_path).exists(), "失败路径不产生备份");
    }

    /// 用户手改坏 settings.json：合并在一切写盘之前失败，两文件终态一致
    /// （同旧）——.env 不先落半截新值（#59 半截状态）。
    #[test]
    fn write_gemini_live_at_fails_before_writing_when_settings_json_is_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        fs::write(&env_path, "GEMINI_API_KEY=sk-old\n").unwrap();
        fs::write(&settings_path, "{oops").unwrap();

        let r = write_gemini_live_at(
            &env_path,
            &settings_path,
            r#"{"env":{"GEMINI_API_KEY":"sk-new"}}"#,
        );
        assert!(r.is_err(), "坏 settings.json 必须失败");
        assert_eq!(
            fs::read_to_string(&env_path).unwrap(),
            "GEMINI_API_KEY=sk-old\n",
            "settings 合并失败不得先写 .env（半截状态）"
        );
        assert_eq!(fs::read_to_string(&settings_path).unwrap(), "{oops");
        assert!(!env_backup_path(&env_path).exists(), "失败路径不得产生备份");
    }

    /// settings 写盘一步失败（备份被占成目录）→ 已写的 .env 回滚到写盘前
    /// 内容；原本不存在的 .env 回滚后被删除。两文件终态一致（#59）。
    #[test]
    fn write_gemini_live_at_rolls_back_env_when_settings_step_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        let old_env = "GEMINI_API_KEY=sk-old\n";
        fs::write(&env_path, old_env).unwrap();
        fs::write(&settings_path, existing_settings("oauth")).unwrap();
        // 让 settings 备份一步失败：settings.json.bak 占成目录 → fs::copy 失败。
        fs::create_dir(tmp.path().join("settings.json.bak")).unwrap();

        let r = write_gemini_live_at(
            &env_path,
            &settings_path,
            r#"{"env":{"GEMINI_API_KEY":"sk-new"}}"#,
        );
        assert!(r.is_err(), "settings 步失败必须报错");
        assert_eq!(
            fs::read_to_string(&env_path).unwrap(),
            old_env,
            ".env 必须回滚到写盘前内容"
        );
        assert_eq!(
            fs::read_to_string(&settings_path).unwrap(),
            existing_settings("oauth"),
            "settings 不得留下半截内容"
        );

        // 原本不存在 .env：回滚后必须删除（不残留新写的空/新文件）。
        let tmp2 = tempfile::tempdir().unwrap();
        let env_path2 = tmp2.path().join(".env");
        let settings_path2 = tmp2.path().join("settings.json");
        fs::write(&settings_path2, existing_settings("oauth")).unwrap();
        fs::create_dir(tmp2.path().join("settings.json.bak")).unwrap();
        let r2 = write_gemini_live_at(
            &env_path2,
            &settings_path2,
            r#"{"env":{"GEMINI_API_KEY":"sk-new"}}"#,
        );
        assert!(r2.is_err());
        assert!(!env_path2.exists(), "原本不存在的 .env 在回滚后必须被删除");
    }

    /// gemini 通用片段经 settings_config 层并入 .env（#50）：apply_snippet 把片段
    /// env 键级补缺失进供应商 settingsConfig（供应商已有键保留），再走既有写盘
    /// 整块写 .env——片段键随整块落地。跑 switch_provider_cmd 的 gemini 分支所调
    /// 的两个纯函数链（apply_snippet → write_gemini_live），验证生产路径。
    #[test]
    fn snippet_env_flows_into_env_file_via_settings_config_layer() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");

        // 供应商已有 GEMINI_API_KEY + 端点；片段补 GEMINI_MODEL（缺失键）。
        let provider_cfg =
            r#"{"env":{"GEMINI_API_KEY":"sk-x","GOOGLE_GEMINI_BASE_URL":"https://gen.dev"}}"#;
        let snippet = r#"{"env":{"GEMINI_MODEL":"gemini-2.5-flash"}}"#;
        let merged = crate::provider::snippet::apply_snippet(provider_cfg, snippet, true).unwrap();

        write_gemini_live_at(&env_path, &settings_path, &merged).unwrap();

        // 片段补的 GEMINI_MODEL 随 .env 整块写落地；供应商已有键保留。
        let env_text = fs::read_to_string(&env_path).unwrap();
        assert!(
            env_text.contains("GEMINI_MODEL=gemini-2.5-flash"),
            "片段 env 并入 .env"
        );
        assert!(env_text.contains("GEMINI_API_KEY=sk-x"), "供应商 env 保留");
        // 供应商赢：片段不得覆盖供应商已有键（这里片段只补缺失，无冲突键）。
        // selectedType 不变量：API Key 版（env 含 GEMINI_API_KEY）。
        let settings = parsed(&fs::read_to_string(&settings_path).unwrap());
        assert_eq!(
            settings["security"]["auth"]["selectedType"],
            serde_json::json!("gemini-api-key")
        );
    }

    /// 停用片段 → apply_snippet 原样返回 → .env 只含供应商 env，片段键不出现。
    #[test]
    fn disabled_snippet_does_not_merge_into_env_file() {
        let tmp = tempfile::tempdir().unwrap();
        let env_path = tmp.path().join(".env");
        let settings_path = tmp.path().join("settings.json");
        let provider_cfg = r#"{"env":{"GEMINI_API_KEY":"sk-x"}}"#;
        let snippet = r#"{"env":{"GEMINI_MODEL":"gemini-2.5-flash"}}"#;
        // enabled=false → apply_snippet 不解析不合并，原样返回。
        let passthrough =
            crate::provider::snippet::apply_snippet(provider_cfg, snippet, false).unwrap();
        assert_eq!(passthrough, provider_cfg);

        write_gemini_live_at(&env_path, &settings_path, &passthrough).unwrap();
        let env_text = fs::read_to_string(&env_path).unwrap();
        assert!(!env_text.contains("GEMINI_MODEL"), "停用片段不并入 .env");
        assert!(env_text.contains("GEMINI_API_KEY=sk-x"));
    }
}
