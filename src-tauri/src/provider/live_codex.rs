//! Codex 写盘（live_codex）：TOML 受控合并 + auth.json 登录态分支。
//!
//! 与 claude 分支同一套「受控合并 / 非受控保留 / 备份 / 原子写」原则：
//! - 目标文件：`~/.codex/config.toml`（TOML）+ `~/.codex/auth.json`（JSON）。
//! - **TOML 受控合并**：供应商快照（settingsConfig.config 的 TOML 文本）里
//!   出现的受控键（见 [`CODEX_CONTROLLED_FIELDS`]）整块替换 live 对应键；
//!   用户手动的 `mcp_servers` / `web_search` / `approval_policy` 等非受控
//!   字段从 live 原样保留（`toml_edit` 重写保留注释与格式）。**绝不整文件
//!   覆盖**——目标是受控键替换，不是把 live 换成快照。
//! - **auth.json**：供应商带非空 `OPENAI_API_KEY`（settingsConfig.auth）
//!   → 受控合并写（只替换 `OPENAI_API_KEY`，登录 token 等非受控字段保留）；
//!   **登录态版**（官方预设无 key）→ 完全不写 auth.json，保留既有 ChatGPT
//!   登录态。
//! - 写前备份 `config.toml.bak`（单份覆盖；auth.json 是凭据/登录态，不
//!   备份）；两文件各自「临时文件 + 改名」原子写；先 auth 后 config，
//!   config 一步失败回滚 auth——任何失败路径都不产生半截状态。
//! - 清洗：写盘前剥 settingsConfig 里的内部 meta 字段（沿用
//!   `live::LIVE_INTERNAL_KEYS` 语义）。
//!
//! `merge_codex_config` 是纯函数（本项目最高价值的测试接缝之一）：输入
//! (当前 live TOML 文本, 目标 TOML 文本) → 输出合并后的 TOML 文本，不碰
//! 文件系统。「非受控字段保留」这个关键不变量靠它落进可测代码。

use std::fs;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::error::{AppError, AppResult};

/// Codex 受控键清单（与 claude 的 `CONTROLLED_FIELDS` 并列，各自是所属
/// 应用写盘的唯一权威）：供应商快照（TOML）里出现这些键 → 整块替换 live
/// 对应键；其余（`mcp_servers` / `web_search` / `approval_policy` 等用户
/// 手动的配置）非受控，切换时原样保留。清单覆盖模型选择（`model` /
/// `model_provider` / `model_providers` / `model_reasoning_effort` /
/// `disable_response_storage`）、凭据覆写（`experimental_bearer_token`）、
/// 模型目录（`model_catalog_json`，快照带了才替换）与 wire 协议
/// （`wire_api`，通常住在 `model_providers` 表里、随表整体替换）。
pub const CODEX_CONTROLLED_FIELDS: &[&str] = &[
    "model",
    "model_provider",
    "model_providers",
    "model_reasoning_effort",
    "disable_response_storage",
    "experimental_bearer_token",
    "model_catalog_json",
    "wire_api",
];

/// `~/.codex` 目录（跨平台统一走 home）。
pub fn codex_config_dir() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".codex"))
}

/// `~/.codex/config.toml` 路径。
pub fn codex_config_path() -> AppResult<PathBuf> {
    Ok(codex_config_dir()?.join("config.toml"))
}

/// `~/.codex/auth.json` 路径（ChatGPT 登录态 + 受控 `OPENAI_API_KEY`）。
pub fn codex_auth_path() -> AppResult<PathBuf> {
    Ok(codex_config_dir()?.join("auth.json"))
}

/// 一次 codex 写盘的受控载荷：从供应商 settingsConfig（`{"auth": ...,
/// "config": "TOML"}` JSON 对象）提取出的写盘内容。`auth_key` 为 `None` 即
/// 登录态版——不写 auth.json。
pub struct CodexSnapshot {
    /// 供应商带的有效 `OPENAI_API_KEY`（trim 后非空）。`None` → 不碰
    /// auth.json，保留既有 ChatGPT 登录态。
    pub auth_key: Option<String>,
    /// 目标 config.toml 文本（受控合并的 target；缺省 = 空串 = 无受控内容）。
    pub config: String,
}

/// 解析供应商 settingsConfig 为写盘载荷：剥内部 meta 字段（沿用
/// `LIVE_INTERNAL_KEYS` 语义），提取 `auth.OPENAI_API_KEY` 与 `config`。
///
/// 边界：空串/纯空白 → 空载荷（无 key 无 config）；非对象 settingsConfig、
/// 非对象 `auth`、非字符串 `OPENAI_API_KEY`（空串除外，视为无 key）、非
/// 字符串 `config` → `Err`（坏配置不能进用户 auth.json / config.toml）。
pub fn parse_codex_settings(settings_config: &str) -> AppResult<CodexSnapshot> {
    let trimmed = settings_config.trim();
    if trimmed.is_empty() {
        return Ok(CodexSnapshot {
            auth_key: None,
            config: String::new(),
        });
    }
    let mut obj = crate::provider::live::parse_object(trimmed, "provider settingsConfig")?;
    // 清洗内部 meta 字段：这些键只供应用自己读，不是 auth.json / config.toml
    // 的合法字段（与 claude 分支同一份清单、同一套语义）。
    if let Some(o) = obj.as_object_mut() {
        for key in crate::provider::live::LIVE_INTERNAL_KEYS {
            o.remove(*key);
        }
    }
    let auth_key = match obj.get("auth") {
        None => None,
        Some(auth) => {
            let auth_obj = auth.as_object().ok_or_else(|| {
                AppError::Config("provider settingsConfig auth is not a JSON object".into())
            })?;
            match auth_obj.get("OPENAI_API_KEY") {
                None => None,
                Some(key) => {
                    let key = key.as_str().ok_or_else(|| {
                        AppError::Config(
                            "provider settingsConfig auth.OPENAI_API_KEY must be a string".into(),
                        )
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
    let config = match obj.get("config") {
        None => String::new(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| {
                AppError::Config("provider settingsConfig config must be a TOML string".into())
            })?
            .to_string(),
    };
    Ok(CodexSnapshot { auth_key, config })
}

/// TOML 受控合并纯函数（最高价值测试接缝）：目标（供应商快照）里出现的
/// [`CODEX_CONTROLLED_FIELDS`] 键整块替换进 live，其余键从 live 原样保留
/// （`toml_edit` 重写保留注释与格式）。不碰文件系统。
///
/// 边界：live / target 为空串或纯空白 → 视为空文档（live 空 = 没有现存
/// 配置可保留；target 空 = 无受控内容）；live 或 target 是非空非法 TOML
/// → `Err`（live 解析不了就没法保留用户手动配置、target 解析不了不能进
/// 用户 config.toml，都宁可失败）。
pub fn merge_codex_config(live: &str, target: &str) -> AppResult<String> {
    let mut doc = parse_toml_or_empty(live, "live config.toml")?;
    let target_doc = parse_toml_or_empty(target, "provider config.toml")?;
    for key in CODEX_CONTROLLED_FIELDS {
        if let Some(item) = target_doc.get(key) {
            doc.as_table_mut().insert(key, item.clone());
        }
    }
    Ok(doc.to_string())
}

/// 解析 TOML 文本为可编辑文档：空串/纯空白 → 空文档；非法 TOML → `Err`。
fn parse_toml_or_empty(text: &str, what: &str) -> AppResult<DocumentMut> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>()
        .map_err(|e| AppError::Config(format!("{what} is not valid TOML: {e}")))
}

/// 构建 auth.json 受控写入载荷：现有内容（缺失 → 空对象）上替换受控键
/// `OPENAI_API_KEY`，其余键（登录 token / auth_mode 等）原样保留。现有
/// 内容不是合法 JSON 对象 → `Err`（解析不了就没法证明能保留既有登录态，
/// 宁可失败）。
fn build_auth_payload(existing: Option<&str>, key: &str) -> AppResult<String> {
    let mut obj: serde_json::Map<String, serde_json::Value> = match existing {
        Some(text) => {
            let v: serde_json::Value = serde_json::from_str(text)
                .map_err(|e| AppError::Config(format!("codex auth.json is not valid JSON: {e}")))?;
            v.as_object()
                .cloned()
                .ok_or_else(|| AppError::Config("codex auth.json is not a JSON object".into()))?
        }
        None => serde_json::Map::new(),
    };
    obj.insert(
        "OPENAI_API_KEY".into(),
        serde_json::Value::String(key.to_string()),
    );
    Ok(serde_json::to_string_pretty(&serde_json::Value::Object(
        obj,
    ))?)
}

/// 切换写盘全流程（薄壳，按序调用）：解析快照 → TOML 受控合并 → 判定
/// auth.json 是否要写 → 两文件都没变化则无操作 → 备份 config → 先原子写
/// auth 后原子写 config（config 一步失败回滚 auth）。
pub fn switch_codex_live(
    config_path: &Path,
    auth_path: &Path,
    settings_config: &str,
) -> AppResult<()> {
    let snapshot = parse_codex_settings(settings_config)?;
    let live = crate::provider::live::read_live_settings(config_path)?;
    let merged = merge_codex_config(&live, &snapshot.config)?;

    // auth.json 现状（缺失 = 本机无登录态也无 key）。登录态版（快照无 key）
    // 完全不写 auth.json；API Key 版在现有内容上合并受控键——登录 token 等
    // 非受控字段原样保留。
    let existing_auth: Option<String> = match fs::read_to_string(auth_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let auth_payload = match &snapshot.auth_key {
        Some(key) => Some(build_auth_payload(existing_auth.as_deref(), key)?),
        None => None,
    };

    // 内容都没变化 → 无操作（不备份、不写盘、不碰 mtime）。trim_end 容忍
    // toml_edit 重写时对结尾换行的归一化。
    let config_changed = merged.trim_end() != live.trim_end();
    let auth_changed = match (&auth_payload, &existing_auth) {
        (Some(payload), Some(existing)) => payload != existing,
        (Some(_), None) => true,
        (None, _) => false,
    };
    if !config_changed && !auth_changed {
        return Ok(());
    }

    // 原子写两文件：先 auth 后 config，config 一步失败回滚 auth。每个文件
    // 自身是临时文件 + 改名，进程中断只留临时文件、不产生半截 config.toml /
    // auth.json。
    if let Some(payload) = &auth_payload {
        crate::provider::live::atomic_write_file(auth_path, payload)?;
    }
    if config_changed {
        let write_result = crate::provider::live::backup_file(config_path)
            .and_then(|()| crate::provider::live::atomic_write_file(config_path, &merged));
        if let Err(e) = write_result {
            if auth_payload.is_some() {
                rollback_auth(auth_path, &existing_auth);
            }
            return Err(e);
        }
    }
    Ok(())
}

/// 回滚 auth.json 到写盘前状态：写盘前存在则还原原文，原本不存在则删除。
/// 回滚自身失败只记录不覆盖主错误——主错误（写 config 失败）才要报告。
fn rollback_auth(auth_path: &Path, existing: &Option<String>) {
    let result = match existing {
        Some(text) => crate::provider::live::atomic_write_file(auth_path, text),
        None => fs::remove_file(auth_path).map_err(AppError::from),
    };
    if let Err(e) = result {
        eprintln!("[vaultone] codex config write failed and auth.json rollback also failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 一份带用户手动配置（非受控字段）的 live config.toml：注释、自定义
    /// 间距、mcp_servers、web_search 都要原样保留。
    fn live_with_uncontrolled() -> String {
        r#"# 用户手动的配置：非受控字段
model = "gpt-5.6"
model_reasoning_effort = "high"

[model_providers.custom]
name = "Old"
base_url = "https://old.dev"
wire_api = "responses"

[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

[web_search]
enabled = true
"#
        .to_string()
    }

    /// 目标快照 TOML（第三方预设形状：model + model_provider + model_providers）。
    fn third_party_target(model: &str, name: &str) -> String {
        format!(
            r#"model_provider = "custom"
model = {model:?}

[model_providers.custom]
name = {name:?}
base_url = "https://api.moonshot.cn/v1"
wire_api = "responses"
requires_openai_auth = true
"#
        )
    }

    /// 按路径取合并结果里的字符串值（测试读值用，不依赖格式）。
    fn get_str(s: &str, path: &[&str]) -> Option<String> {
        let doc: DocumentMut = s.parse().ok()?;
        let mut cur = doc.get(path[0])?;
        for key in &path[1..] {
            cur = cur.get(*key)?;
        }
        cur.as_str().map(str::to_string)
    }

    fn parsed_doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }

    #[test]
    fn controlled_fields_replaced_uncontrolled_preserved() {
        let live = live_with_uncontrolled();
        let target = third_party_target("kimi-k2.7-code", "kimi");
        let merged = merge_codex_config(&live, &target).unwrap();
        // 受控键整块替换。
        assert_eq!(
            get_str(&merged, &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );
        assert_eq!(
            get_str(&merged, &["model_providers", "custom", "name"]).as_deref(),
            Some("kimi")
        );
        assert_eq!(
            get_str(&merged, &["model_providers", "custom", "base_url"]).as_deref(),
            Some("https://api.moonshot.cn/v1")
        );
        // 非受控字段原样保留。
        assert_eq!(
            get_str(&merged, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx")
        );
        assert_eq!(
            get_str(&merged, &["web_search", "enabled"]).as_deref(),
            Some("true")
        );
        // 受控键但目标没带 → 保留 live 原值（model_reasoning_effort）。
        assert_eq!(
            get_str(&merged, &["model_reasoning_effort"]).as_deref(),
            Some("high")
        );
        // 注释保留（toml_edit 重写不丢注释）。
        assert!(merged.contains("用户手动的配置"), "注释必须保留: {merged}");
    }

    #[test]
    fn target_uncontrolled_fields_are_ignored_not_written() {
        let live = live_with_uncontrolled();
        // 目标也带了 mcp_servers / web_search——非受控，绝不能覆盖 live 的。
        let target = r#"model = "gpt-5.6"

[mcp_servers.filesystem]
command = "python"
args = []

[web_search]
enabled = false
"#;
        let merged = merge_codex_config(&live, target).unwrap();
        assert_eq!(
            get_str(&merged, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx"),
            "live 的 mcp_servers 保留，目标的被忽略"
        );
        assert_eq!(
            get_str(&merged, &["web_search", "enabled"]).as_deref(),
            Some("true"),
            "live 的 web_search 保留"
        );
    }

    #[test]
    fn empty_live_merges_to_target_controlled_only() {
        let target = third_party_target("kimi-k2.7-code", "kimi");
        let merged = merge_codex_config("", &target).unwrap();
        let doc = parsed_doc(&merged);
        assert_eq!(
            get_str(&merged, &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );
        assert!(
            doc.get("mcp_servers").is_none(),
            "live 为空时目标里的非受控字段也不得写入"
        );
        assert!(doc.get("web_search").is_none());
    }

    #[test]
    fn empty_target_keeps_live_unchanged() {
        let live = live_with_uncontrolled();
        let merged = merge_codex_config(&live, "").unwrap();
        // 目标没有受控内容 → live 原样。
        assert_eq!(get_str(&merged, &["model"]).as_deref(), Some("gpt-5.6"));
        assert_eq!(
            get_str(&merged, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx")
        );
    }

    #[test]
    fn comment_and_format_preserved_on_untouched_lines() {
        let live = r#"# 用户手动的配置
model   =   "gpt-5.6"

[model_providers.custom]
name = "Old"
"#;
        // 目标只动 model_providers，不碰 model——model 行要逐字节保留。
        let target = r#"[model_providers.custom]
name = "New"
"#;
        let merged = merge_codex_config(live, target).unwrap();
        assert!(
            merged.contains("model   =   \"gpt-5.6\""),
            "未受控行的格式必须逐字节保留: {merged}"
        );
        assert!(
            merged.contains("# 用户手动的配置"),
            "注释必须保留: {merged}"
        );
        assert_eq!(
            get_str(&merged, &["model_providers", "custom", "name"]).as_deref(),
            Some("New")
        );
    }

    #[test]
    fn invalid_live_toml_is_an_error() {
        let r = merge_codex_config("model = [1,2", r#"model = "m""#);
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "live 非法 TOML 必须失败——解析不了就没法保留用户手动配置"
        );
    }

    #[test]
    fn invalid_target_toml_is_an_error() {
        let r = merge_codex_config("", "not toml {");
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "目标非法 TOML 必须失败——坏配置不能进用户 config.toml"
        );
    }

    #[test]
    fn parse_settings_extracts_key_and_config() {
        let s = parse_codex_settings(
            r#"{"auth":{"OPENAI_API_KEY":" sk-123 "},"config":"model = \"m\""}"#,
        )
        .unwrap();
        assert_eq!(s.auth_key.as_deref(), Some("sk-123"), "key 要 trim");
        assert_eq!(s.config, r#"model = "m""#);
    }

    #[test]
    fn parse_settings_login_state_versions_have_no_key() {
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
    fn parse_settings_strips_internal_meta_keys() {
        let s = parse_codex_settings(
            r#"{"api_format":"openai","apiFormat":"openai","openrouter_compat_mode":true,"openrouterCompatMode":true,"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        )
        .unwrap();
        assert_eq!(s.auth_key.as_deref(), Some("sk-1"));
        assert_eq!(s.config, r#"model = "m""#);
    }

    #[test]
    fn parse_settings_rejects_bad_shapes() {
        // 非对象 settingsConfig。
        assert!(parse_codex_settings("[1,2]").is_err());
        assert!(parse_codex_settings(r#""just a string""#).is_err());
        // 非对象 auth。
        assert!(parse_codex_settings(r#"{"auth":"sk-plain"}"#).is_err());
        // 非字符串 OPENAI_API_KEY。
        assert!(parse_codex_settings(r#"{"auth":{"OPENAI_API_KEY":123}}"#).is_err());
        // 非字符串 config。
        assert!(parse_codex_settings(r#"{"config":123}"#).is_err());
    }

    #[test]
    fn empty_settings_is_an_empty_snapshot() {
        for raw in ["", "   "] {
            let s = parse_codex_settings(raw).unwrap();
            assert_eq!(s.auth_key, None);
            assert_eq!(s.config, "");
        }
    }

    /// 临时目录里放好 auth.json（模拟用户 ChatGPT 登录态）+ config.toml。
    fn seed(tmp: &Path, auth: Option<&str>, config: Option<&str>) -> (PathBuf, PathBuf) {
        let auth_path = tmp.join("auth.json");
        let config_path = tmp.join("config.toml");
        if let Some(a) = auth {
            fs::write(&auth_path, a).unwrap();
        }
        if let Some(c) = config {
            fs::write(&config_path, c).unwrap();
        }
        (config_path, auth_path)
    }

    #[test]
    fn login_state_version_does_not_touch_auth_json() {
        let tmp = tempfile::tempdir().unwrap();
        let login = r#"{"tokens":{"id_token":"abc"},"auth_mode":"login"}"#;
        let (config_path, auth_path) = seed(tmp.path(), Some(login), Some("model = \"gpt-5.6\"\n"));

        // 登录态版（无 key、无 config 内容）：两文件都要原样保留。
        switch_codex_live(&config_path, &auth_path, r#"{"auth":{}}"#).unwrap();
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), login);
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "model = \"gpt-5.6\"\n"
        );
        assert!(
            !tmp.path().join("config.toml.bak").exists(),
            "无变化不得触发备份"
        );

        // auth.json 原本不存在 → 保持不存在（不创建空文件）。
        let tmp2 = tempfile::tempdir().unwrap();
        let (config_path2, auth_path2) = seed(tmp2.path(), None, None);
        switch_codex_live(
            &config_path2,
            &auth_path2,
            r#"{"auth":{"OPENAI_API_KEY":""}}"#,
        )
        .unwrap();
        assert!(!auth_path2.exists(), "登录态版不得创建 auth.json");
        assert!(!config_path2.exists(), "空快照不得创建 config.toml");
    }

    #[test]
    fn api_key_version_writes_auth_json_and_preserves_login() {
        let tmp = tempfile::tempdir().unwrap();
        let login = r#"{"tokens":{"id_token":"abc"},"auth_mode":"login"}"#;
        let (config_path, auth_path) = seed(tmp.path(), Some(login), Some("model = \"gpt-5.6\"\n"));

        switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"auth":{"OPENAI_API_KEY":"sk-123"},"config":"model = \"kimi-k2.7-code\""}"#,
        )
        .unwrap();

        let auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], serde_json::json!("sk-123"));
        assert_eq!(
            auth["auth_mode"],
            serde_json::json!("login"),
            "登录态字段保留"
        );
        assert!(auth["tokens"].is_object(), "登录 token 保留");
        // config 一并写完（语义断言，不依赖 toml_edit 的字节格式）。
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            get_str(&written, &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );
    }

    #[test]
    fn api_key_version_creates_auth_json_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, None);
        switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        )
        .unwrap();
        let auth: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&auth_path).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], serde_json::json!("sk-1"));
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(get_str(&written, &["model"]).as_deref(), Some("m"));
    }

    #[test]
    fn existing_auth_json_invalid_is_an_error_before_any_write() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) =
            seed(tmp.path(), Some("{oops"), Some("model = \"gpt-5.6\"\n"));
        let r = switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        );
        assert!(
            r.is_err(),
            "无法解析的 auth.json 不能覆盖——登录态可能在里面"
        );
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), "{oops");
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "model = \"gpt-5.6\"\n",
            "写失败不得留下半截 config"
        );
    }

    #[test]
    fn backup_created_when_config_changes_and_not_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(
            tmp.path(),
            None,
            Some("model = \"gpt-5.6\"\n\n[mcp_servers.filesystem]\ncommand = \"npx\"\n"),
        );
        switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"config":"model = \"kimi-k2.7-code\""}"#,
        )
        .unwrap();
        let bak = tmp.path().join("config.toml.bak");
        assert!(bak.exists(), "config 变化必须备份");
        assert!(
            fs::read_to_string(&bak).unwrap().contains("gpt-5.6"),
            ".bak 是写盘前的 live 快照"
        );
        assert_eq!(
            get_str(&fs::read_to_string(&config_path).unwrap(), &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );

        // 再次切到同一内容（无 trailing newline 的输入也视为无变化）→ 无
        // 操作、无备份、内容不变。
        let before = fs::read_to_string(&config_path).unwrap();
        fs::remove_file(&bak).unwrap();
        switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"config":"model = \"kimi-k2.7-code\""}"#,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);
        assert!(!bak.exists(), "内容无变化不得触发备份");
    }

    #[test]
    fn config_missing_creates_file_without_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, None);
        switch_codex_live(&config_path, &auth_path, r#"{"config":"model = \"m\""}"#).unwrap();
        assert!(config_path.exists());
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(get_str(&written, &["model"]).as_deref(), Some("m"));
        assert!(
            !tmp.path().join("config.toml.bak").exists(),
            "live 原本不存在 → 无备份"
        );
    }

    #[test]
    fn config_step_failure_rolls_back_auth() {
        let tmp = tempfile::tempdir().unwrap();
        let old_auth = r#"{"tokens":{"id_token":"abc"}}"#;
        let (config_path, auth_path) =
            seed(tmp.path(), Some(old_auth), Some("model = \"gpt-5.6\"\n"));
        // 让备份一步失败：把 config.toml.bak 路径占成目录 → fs::copy 失败。
        fs::create_dir(tmp.path().join("config.toml.bak")).unwrap();

        let r = switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        );
        assert!(r.is_err(), "config 一步失败必须报错");
        assert_eq!(
            fs::read_to_string(&auth_path).unwrap(),
            old_auth,
            "auth 已写出的部分必须回滚到写盘前"
        );
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "model = \"gpt-5.6\"\n",
            "config 不得留下半截内容"
        );
    }

    #[test]
    fn config_step_failure_removes_newly_created_auth() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, Some("model = \"gpt-5.6\"\n"));
        fs::create_dir(tmp.path().join("config.toml.bak")).unwrap();

        let r = switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        );
        assert!(r.is_err());
        assert!(
            !auth_path.exists(),
            "原本不存在的 auth.json 在回滚后必须被删除"
        );
    }

    #[test]
    fn auth_write_failure_leaves_config_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, Some("model = \"gpt-5.6\"\n"));
        // auth.json 位置占成目录 → 原子写失败（rename 到目录上必败）。
        fs::create_dir(&auth_path).unwrap();

        let r = switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"auth":{"OPENAI_API_KEY":"sk-1"},"config":"model = \"m\""}"#,
        );
        assert!(r.is_err(), "auth 写失败必须报错");
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            "model = \"gpt-5.6\"\n",
            "auth 失败不得先写 config"
        );
        assert!(
            !tmp.path().join("config.toml.bak").exists(),
            "auth 失败不得触发备份"
        );
    }

    #[test]
    fn switch_is_noop_when_nothing_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, None);
        // 第一次切换写出 merged；同样的快照再切一次 → 合并结果字节相同 →
        // 无操作（不备份、不重写、不碰 mtime）。用「同一输入两连切」保证
        // 判定不依赖 toml_edit 的字节渲染细节。
        let target = r#"{"config":"model = \"gpt-5.6\""}"#;
        switch_codex_live(&config_path, &auth_path, target).unwrap();
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(get_str(&written, &["model"]).as_deref(), Some("gpt-5.6"));
        switch_codex_live(&config_path, &auth_path, target).unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), written);
        assert!(
            !tmp.path().join("config.toml.bak").exists(),
            "全无变化 → 不备份"
        );
    }

    #[test]
    fn codex_paths_point_at_home_codex_dir() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(codex_config_dir().unwrap(), home.join(".codex"));
        assert_eq!(
            codex_config_path().unwrap(),
            home.join(".codex").join("config.toml")
        );
        assert_eq!(
            codex_auth_path().unwrap(),
            home.join(".codex").join("auth.json")
        );
    }
}
