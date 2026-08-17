//! Codex 写盘（live_codex）：TOML 受控合并 + auth.json 登录态分支。
//!
//! 与 claude 分支同一套「受控合并 / 非受控保留 / 备份 / 原子写」原则：
//! - 目标文件：`~/.codex/config.toml`（TOML）+ `~/.codex/auth.json`（JSON）。
//! - **TOML 受控合并**：供应商快照（settingsConfig.config 的 TOML 文本）里
//!   出现的受控键（见 [`CODEX_CONTROLLED_FIELDS`]）整块替换 live 对应键；
//!   快照缺失的受控键从 live **撤除**（ADR-0010 受控轴：新供应商赢——否则
//!   旧供应商身份键残留、切换静默失效，官方登录态版快照为空正依赖撤除回到
//!   无身份配置）；用户手动的 `mcp_servers` / `web_search` / `approval_policy`
//!   等非受控字段从 live 原样保留（`toml_edit` 重写保留注释与格式）。**绝不
//!   整文件覆盖**——目标是受控键替换，不是把 live 换成快照。
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
/// 对应键；快照缺失 → 从 live 撤除（受控轴「新供应商赢」，防旧供应商身份
/// 残留）；其余（`mcp_servers` / `web_search` / `approval_policy` 等用户
/// 手动的配置）非受控，切换时原样保留。清单覆盖模型选择（`model` /
/// `model_provider` / `model_providers` / `model_reasoning_effort` /
/// `disable_response_storage`）、凭据覆写（`experimental_bearer_token`）、
/// 模型目录（`model_catalog_json`）与 wire 协议（`wire_api`，通常住在
/// `model_providers` 表里、随表整体替换）。
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

/// 解析供应商 settingsConfig 为写盘载荷（内部 meta 字段已在公共解析层
/// `parse_and_strip_settings` 剥除），提取 `auth.OPENAI_API_KEY` 与 `config`。
///
/// 边界：空串/纯空白 → 空载荷（无 key 无 config）；非对象 settingsConfig、
/// 非对象 `auth`、非字符串 `OPENAI_API_KEY`（空串除外，视为无 key）、非
/// 字符串 `config` → `Err`（坏配置不能进用户 auth.json / config.toml）。
pub fn parse_codex_settings(settings_config: &str) -> AppResult<CodexSnapshot> {
    let Some(obj) = crate::provider::live::parse_and_strip_settings(settings_config)? else {
        return Ok(CodexSnapshot {
            auth_key: None,
            config: String::new(),
        });
    };
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
    let config = crate::provider::live::config_toml_field(&obj)?;
    Ok(CodexSnapshot { auth_key, config })
}

/// TOML 受控合并纯函数（最高价值测试接缝）：目标（供应商快照）里出现的
/// [`CODEX_CONTROLLED_FIELDS`] 键整块替换进 live，快照缺失的受控键从 live
/// 撤除；其余键从 live 原样保留（`toml_edit` 重写保留注释与格式）。不碰
/// 文件系统。
///
/// 边界：live / target 为空串或纯空白 → 视为空文档（live 空 = 没有现存
/// 配置可保留；target 空 = 无受控内容）；live 或 target 是非空非法 TOML
/// → `Err`（live 解析不了就没法保留用户手动配置、target 解析不了不能进
/// 用户 config.toml，都宁可失败）。
pub fn merge_codex_config(live: &str, target: &str) -> AppResult<String> {
    let mut doc = crate::provider::live::parse_toml_or_empty(live, "live config.toml")?;
    let target_doc = crate::provider::live::parse_toml_or_empty(target, "provider config.toml")?;
    for key in CODEX_CONTROLLED_FIELDS {
        match target_doc.get(key) {
            Some(item) => {
                doc.as_table_mut().insert(key, item.clone());
            }
            None => {
                // 目标缺失 → 从 live 撤除（ADR-0010 受控轴：新供应商赢，否则
                // 旧供应商的身份键残留、切换静默失效——第三方 → 官方登录态版
                // 必须清掉旧 base_url / token）。
                doc.as_table_mut().remove(key);
            }
        }
    }
    Ok(doc.to_string())
}

/// 写盘层片段补缺失纯函数：在 `merge_codex_config` 产出的合并结果上，把片段
/// 的**非受控**键补进去（live 已有则保留、递归进子表）。身份键（见
/// [`CODEX_CONTROLLED_FIELDS`]）归供应商，片段不得携带——含身份键 → `Err`
/// （与 [`validate_codex_snippet`] 同款拒绝，set 命令提前拦 + 合并纯函数兜底，
/// 防绕过 set 的路径）。
///
/// 为什么在写盘层而非 settings_config 层：`merge_codex_config` 只搬身份键、
/// 丢弃 target 其余键；在 settings_config 层合片段会被写盘白名单滤掉 → 片段
/// 零效果（见 ADR-0010）。故片段必须在 merge 之后、写盘之前补到 live doc。
/// 凭据键**不禁**——codex 片段写 `mcp_servers` 等（独立进程、不经 LLM 端点），
/// 用户要共享带 token 的 MCP。
///
/// 边界：`snippet` 空串/纯空白 → 视为空文档（无操作）；非空非法 TOML → `Err`；
/// 含身份键 → `Err`。`merged` 来自 `merge_codex_config`，假定合法；非空非法
/// → `Err`（防御）。
pub fn merge_codex_snippet(merged: &str, snippet: &str) -> AppResult<String> {
    crate::provider::live::merge_toml_snippet(merged, snippet, "codex", codex_identity_hit)
}

/// codex 片段校验（set 命令用）：合法 TOML；不得含受控身份键。凭据键不禁
/// （codex 片段写 `mcp_servers` 等、不经 LLM 端点，见 ADR-0010）。骨架与
/// grok 共用（`live::validate_toml_snippet`），只有身份键谓词是 codex 自己的。
pub fn validate_codex_snippet(snippet: &str) -> AppResult<()> {
    crate::provider::live::validate_toml_snippet(snippet, "codex", codex_identity_hit)
}

/// codex 身份键命中描述（报错用，#55：键名细节只出现在校验报错里）：
/// [`CODEX_CONTROLLED_FIELDS`] 里第一个出现在片段里的键。
fn codex_identity_hit(doc: &DocumentMut) -> Option<String> {
    CODEX_CONTROLLED_FIELDS
        .iter()
        .find(|key| doc.get(key).is_some())
        .map(|key| format!("`{key}`"))
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
/// auth 后原子写 config（事务原语 `backup_file` / `atomic_write_file` /
/// 无变化判定 / 回滚收口在 `live`，见 [`crate::provider::live::commit_live_file`]）。
pub fn switch_codex_live(
    config_path: &Path,
    auth_path: &Path,
    settings_config: &str,
    snippet: &str,
) -> AppResult<()> {
    let snapshot = parse_codex_settings(settings_config)?;
    let live = crate::provider::live::read_live_settings(config_path)?;
    let mut merged = merge_codex_config(&live, &snapshot.config)?;
    // 写盘层补片段：merge_codex_config 只搬身份键、丢弃其余，故片段必须在此补
    // （settings_config 层合会被白名单滤掉→零效果，见 ADR-0010）。片段空则跳过。
    if !snippet.trim().is_empty() {
        merged = merge_codex_snippet(&merged, snippet)?;
    }

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

    // 内容都没变化 → 无操作（不备份、不写盘、不碰 mtime）。auth 是凭据文件、
    // 本应用自己序列化，用精确比较；config 走 trim_end（容忍 toml_edit 重写
    // 对结尾换行的归一化）。
    let config_unchanged = crate::provider::live::content_unchanged(&live, &merged);
    let auth_unchanged = match (&auth_payload, &existing_auth) {
        (Some(payload), Some(existing)) => payload == existing,
        (Some(_), None) => false,
        (None, _) => true,
    };
    if config_unchanged && auth_unchanged {
        return Ok(());
    }

    // 原子写两文件：先 auth 后 config，config 一步失败回滚 auth。每个文件
    // 自身是临时文件 + 改名，进程中断只留临时文件、不产生半截 config.toml /
    // auth.json。
    let auth_written = match (&auth_payload, auth_unchanged) {
        (Some(payload), false) => {
            crate::provider::live::atomic_write_file(auth_path, payload)?;
            true
        }
        _ => false,
    };
    if let Err(e) = crate::provider::live::commit_live_file(config_path, &merged, config_unchanged)
    {
        if auth_written {
            crate::provider::live::rollback_side_file(
                auth_path,
                &existing_auth,
                "codex config write failed and auth.json rollback",
            );
        }
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use toml_edit::DocumentMut;

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
        cur.as_str()
            .map(str::to_string)
            .or_else(|| cur.as_bool().map(|b| b.to_string()))
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
        // 受控键但目标没带 → 从 live 撤除（model_reasoning_effort；受控轴
        // 「新供应商赢」，缺失即撤、不保留旧值）。
        assert!(
            get_str(&merged, &["model_reasoning_effort"]).is_none(),
            "目标缺失的受控键必须撤除，不得残留旧供应商的值"
        );
        // 注释保留针对「未替换的键」成立（见 comment_and_format_preserved_on_
        // untouched_lines）。fixture 顶部 `# 用户手动的配置` 是被替换的 model
        // 键的 leading decor——target 带了 model（受控键），整键替换连同其注释
        // 一起换掉，符合「受控键整块替换」语义；非受控块（mcp_servers /
        // web_search）的原样保留已由上面的断言守住。
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
    fn empty_target_removes_controlled_keeps_uncontrolled() {
        let live = live_with_uncontrolled();
        let merged = merge_codex_config(&live, "").unwrap();
        // 目标没有受控内容 → live 的受控键全部撤除（官方登录态版快照为空，
        // 切换 = 回到无身份配置；旧供应商的 model 残留会让切换静默失效）。
        assert!(
            get_str(&merged, &["model"]).is_none(),
            "空目标必须撤除 live 的受控键 model"
        );
        assert!(
            get_str(&merged, &["model_reasoning_effort"]).is_none(),
            "空目标必须撤除 live 的受控键 model_reasoning_effort"
        );
        assert!(
            parsed_doc(&merged).get("model_providers").is_none(),
            "空目标必须撤除 live 的受控键 model_providers"
        );
        // 非受控字段保留。
        assert_eq!(
            get_str(&merged, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx")
        );
        assert_eq!(
            get_str(&merged, &["web_search", "enabled"]).as_deref(),
            Some("true")
        );
    }

    /// #58 主场景：第三方 → 官方登录态版切换，旧身份键（含凭据覆写）全部
    /// 撤除；官方 → 第三方不回归。
    #[test]
    fn third_party_to_official_withdraws_all_identity_keys() {
        let live = format!(
            "{}\nexperimental_bearer_token = \"sk-old\"\nmodel_provider = \"custom\"\n",
            third_party_target("kimi-k2.7-code", "kimi")
        );
        // 官方登录态版预设 settingsConfig 为空对象 → 空 config。
        let merged = merge_codex_config(&live, "").unwrap();
        let doc = parsed_doc(&merged);
        for key in CODEX_CONTROLLED_FIELDS {
            assert!(
                doc.get(key).is_none(),
                "官方登录态版切换后受控键 {key} 必须撤除"
            );
        }

        // 官方 → 第三方：目标携带 → 替换（不回归）。
        let back =
            merge_codex_config(&merged, &third_party_target("kimi-k2.7-code", "kimi")).unwrap();
        assert_eq!(
            get_str(&back, &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );
        assert_eq!(
            get_str(&back, &["model_providers", "custom", "base_url"]).as_deref(),
            Some("https://api.moonshot.cn/v1")
        );
    }

    #[test]
    fn comment_and_format_preserved_on_untouched_lines() {
        // 用非受控键（approval_policy）验格式逐字节保留：受控键在目标缺失时
        // 会被撤除，不适合作为「未触碰行」的样本。
        let live = r#"# 用户手动的配置
approval_policy   =   "on-request"

[model_providers.custom]
name = "Old"
"#;
        // 目标只动 model_providers，不碰 approval_policy——该行要逐字节保留。
        let target = r#"[model_providers.custom]
name = "New"
"#;
        let merged = merge_codex_config(live, target).unwrap();
        assert!(
            merged.contains("approval_policy   =   \"on-request\""),
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

        // 登录态版（无 key、无 config 内容）：auth.json 原样保留；config 的
        // 受控键撤除（model 移除——切到官方登录态 = 回到无身份配置）。
        switch_codex_live(&config_path, &auth_path, r#"{"auth":{}}"#, "").unwrap();
        assert_eq!(fs::read_to_string(&auth_path).unwrap(), login);
        let written = fs::read_to_string(&config_path).unwrap();
        assert!(
            get_str(&written, &["model"]).is_none(),
            "登录态版切换撤除受控键: {written}"
        );
        let bak = tmp.path().join("config.toml.bak");
        assert!(bak.exists(), "config 变化（撤除受控键）必须备份");
        assert!(
            fs::read_to_string(&bak).unwrap().contains("gpt-5.6"),
            ".bak 是写盘前的 live 快照"
        );

        // auth.json 原本不存在 → 保持不存在（不创建空文件）；config 也不
        // 存在 → 空 live 合空目标无变化，不创建。
        let tmp2 = tempfile::tempdir().unwrap();
        let (config_path2, auth_path2) = seed(tmp2.path(), None, None);
        switch_codex_live(
            &config_path2,
            &auth_path2,
            r#"{"auth":{"OPENAI_API_KEY":""}}"#,
            "",
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
            "",
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
            "",
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
            "",
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
            "",
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
            "",
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);
        assert!(!bak.exists(), "内容无变化不得触发备份");
    }

    #[test]
    fn config_missing_creates_file_without_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, None);
        switch_codex_live(
            &config_path,
            &auth_path,
            r#"{"config":"model = \"m\""}"#,
            "",
        )
        .unwrap();
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
            "",
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
            "",
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
            "",
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
        switch_codex_live(&config_path, &auth_path, target, "").unwrap();
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(get_str(&written, &["model"]).as_deref(), Some("gpt-5.6"));
        switch_codex_live(&config_path, &auth_path, target, "").unwrap();
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

    #[test]
    fn snippet_fills_missing_top_level_key() {
        // merged（merge_codex_config 产物）没有 [tui]，片段补上。
        let merged = r#"model = "kimi-k2.7-code"
"#;
        let snippet = r#"[tui]
theme = "dark"
"#;
        let out = merge_codex_snippet(merged, snippet).unwrap();
        assert_eq!(get_str(&out, &["model"]).as_deref(), Some("kimi-k2.7-code"));
        assert_eq!(get_str(&out, &["tui", "theme"]).as_deref(), Some("dark"));
    }

    #[test]
    fn snippet_does_not_override_existing_non_table() {
        // live 已有非受控标量 approval_policy → 片段不覆盖。
        let merged = r#"approval_policy = "oncalls""#;
        let snippet = r#"approval_policy = "never""#;
        let out = merge_codex_snippet(merged, snippet).unwrap();
        assert_eq!(
            get_str(&out, &["approval_policy"]).as_deref(),
            Some("oncalls"),
            "live 已有的键不覆盖"
        );
    }

    #[test]
    fn snippet_mcp_servers_coexist_and_live_wins_on_shared() {
        // merged 已有 [mcp_servers.filesystem] + [mcp_servers.shared].command="live"；
        // 片段带 [mcp_servers.github]（live 没有→补）+ [mcp_servers.shared]（live
        // 有→递归：command live 赢，片段独有的 new_field 补上）。
        let merged = r#"[mcp_servers.filesystem]
command = "npx"

[mcp_servers.shared]
command = "live"
"#;
        let snippet = r#"[mcp_servers.github]
command = "npx"
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_xxx" }

[mcp_servers.shared]
command = "snippet"
new_field = "from-snippet"
"#;
        let out = merge_codex_snippet(merged, snippet).unwrap();
        // filesystem 保留、github 补上（含凭据，不禁）。
        assert_eq!(
            get_str(&out, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx")
        );
        assert_eq!(
            get_str(&out, &["mcp_servers", "github", "command"]).as_deref(),
            Some("npx")
        );
        assert_eq!(
            get_str(
                &out,
                &[
                    "mcp_servers",
                    "github",
                    "env",
                    "GITHUB_PERSONAL_ACCESS_TOKEN"
                ]
            )
            .as_deref(),
            Some("ghp_xxx"),
            "mcp_servers 含凭据允许（不经 LLM 端点）"
        );
        // shared：command live 赢，new_field 补上。
        assert_eq!(
            get_str(&out, &["mcp_servers", "shared", "command"]).as_deref(),
            Some("live"),
            "同 server 键 live 已有 → 不覆盖"
        );
        assert_eq!(
            get_str(&out, &["mcp_servers", "shared", "new_field"]).as_deref(),
            Some("from-snippet"),
            "递归补缺失：片段独有的子键补上"
        );
    }

    #[test]
    fn snippet_with_identity_key_is_rejected() {
        // 片段含受控身份键 → Err（身份键归供应商）。
        assert!(merge_codex_snippet("", r#"model = "x""#).is_err());
        assert!(merge_codex_snippet(
            "",
            r#"[model_providers.foo]
name = "x"
"#
        )
        .is_err());
        assert!(merge_codex_snippet("", r#"experimental_bearer_token = "t""#).is_err());
    }

    #[test]
    fn empty_snippet_is_noop_for_merge() {
        let merged = r#"model = "m"
[mcp_servers.filesystem]
command = "npx"
"#;
        for empty in ["", "   ", "\n"] {
            let out = merge_codex_snippet(merged, empty).unwrap();
            assert_eq!(
                get_str(&out, &["mcp_servers", "filesystem", "command"]).as_deref(),
                Some("npx")
            );
        }
    }

    #[test]
    fn invalid_snippet_toml_is_error_for_merge() {
        assert!(merge_codex_snippet("model = \"m\"", "not toml {").is_err());
    }

    #[test]
    fn snippet_fill_missing_is_idempotent() {
        // 同一片段连补两次 → 结果不变（只补缺失，不重复追加）。
        let merged = r#"model = "m""#;
        let snippet = r#"[tui]
theme = "dark"
"#;
        let once = merge_codex_snippet(merged, snippet).unwrap();
        let twice = merge_codex_snippet(&once, snippet).unwrap();
        assert_eq!(once, twice);
        assert_eq!(get_str(&twice, &["tui", "theme"]).as_deref(), Some("dark"));
    }

    #[test]
    fn validate_codex_snippet_accepts_shared_keys_and_rejects_identity() {
        // 合法：非受控共享键（含 mcp_servers 带凭据）。
        assert!(validate_codex_snippet(
            r#"[tui]
theme = "dark""#
        )
        .is_ok());
        assert!(validate_codex_snippet(
            r#"[mcp_servers.github]
command = "npx"
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_x" }
"#
        )
        .is_ok());
        assert!(validate_codex_snippet("").is_ok());

        // 拒绝：受控身份键。
        for bad in [
            r#"model = "x""#,
            r#"model_provider = "x""#,
            r#"[model_providers.foo]
name = "x""#,
            r#"experimental_bearer_token = "t""#,
            r#"wire_api = "responses""#,
        ] {
            assert!(
                validate_codex_snippet(bad).is_err(),
                "身份键必须拒绝: {bad}"
            );
        }
        // 非法 TOML。
        assert!(validate_codex_snippet("not toml {").is_err());
    }

    #[test]
    fn switch_writes_snippet_through_write_layer_and_is_idempotent() {
        // 写盘薄壳：贯穿 switch_codex_live(.., snippet) 生产路径。身份键 model
        // 由供应商接管（old→kimi）；片段的非受控键补缺失——[tui] theme live 赢
        // （保留 light）、片段独有子键补上、[mcp_servers] 新增（含凭据不禁）。
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(
            tmp.path(),
            None,
            Some("model = \"old\"\n[tui]\ntheme = \"light\"\n"),
        );
        let target = r#"{"config":"model = \"kimi-k2.7-code\""}"#;
        let snippet = "[tui]\ntop_output_style = \"compact\"\n[mcp_servers.github]\ncommand = \"npx\"\nenv = { GITHUB_PERSONAL_ACCESS_TOKEN = \"ghp_x\" }\n";

        switch_codex_live(&config_path, &auth_path, target, snippet).unwrap();
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            get_str(&written, &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );
        assert_eq!(
            get_str(&written, &["tui", "theme"]).as_deref(),
            Some("light"),
            "live 已有的片段不覆盖"
        );
        assert_eq!(
            get_str(&written, &["tui", "top_output_style"]).as_deref(),
            Some("compact"),
            "片段独有的子键补上"
        );
        assert_eq!(
            get_str(&written, &["mcp_servers", "github", "command"]).as_deref(),
            Some("npx")
        );
        assert_eq!(
            get_str(
                &written,
                &[
                    "mcp_servers",
                    "github",
                    "env",
                    "GITHUB_PERSONAL_ACCESS_TOKEN"
                ]
            )
            .as_deref(),
            Some("ghp_x"),
            "mcp_servers 凭据允许进片段"
        );
        let bak = tmp.path().join("config.toml.bak");
        assert!(bak.exists(), "config 变化必须备份");

        // 幂等：删 .bak 再切同一份 → 合并结果字节相同 → 无操作（不重写、不备份）。
        fs::remove_file(&bak).unwrap();
        switch_codex_live(&config_path, &auth_path, target, snippet).unwrap();
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            written,
            "连切两次字节不变"
        );
        assert!(!bak.exists(), "无变化不得重复备份");
    }

    #[test]
    fn switch_rejects_identity_key_at_write_layer() {
        // set 命令已提前拦身份键；这里兜底——绕过 set 直接调写盘层，片段含身份键
        // 仍 Err，且发生在写盘前（不留下半截 config）。
        let tmp = tempfile::tempdir().unwrap();
        let (config_path, auth_path) = seed(tmp.path(), None, None);
        let target = r#"{"config":"model = \"m\""}"#;
        assert!(switch_codex_live(&config_path, &auth_path, target, r#"model = "x""#).is_err());
        assert!(switch_codex_live(
            &config_path,
            &auth_path,
            target,
            "[model_providers.foo]\nname = \"x\"\n"
        )
        .is_err());
        assert!(!config_path.exists(), "身份键拒绝不得写出 config");
    }

    #[test]
    fn snippet_merge_preserves_comments_and_key_order() {
        // #49 验收：片段合并保留注释与键序（toml_edit 键级编辑，不整文档重写）。
        // 把这条不变量落进测试——换库或换合并器时不会被静默破坏（architecture.md）。
        // 注：受控身份键（model）被供应商整块替换时，紧贴该键的注释会随之让位
        // （身份键归供应商，见 ADR-0010）——这是预期行为，本测只验**非受控**键上
        // 的注释/键序保留（用户手写的偏好区不被片段合并破坏）。
        let tmp = tempfile::tempdir().unwrap();
        let live = "model = \"old\"\n\n# 共享偏好\n[tui]\n# 主题\ntext = \"light\"\n";
        let (config_path, auth_path) = seed(tmp.path(), None, Some(live));
        let target = r#"{"config":"model = \"kimi-k2.7-code\""}"#;
        let snippet =
            "[tui]\ntop_output_style = \"compact\"\n[mcp_servers.github]\ncommand = \"npx\"\n";

        switch_codex_live(&config_path, &auth_path, target, snippet).unwrap();
        let written = fs::read_to_string(&config_path).unwrap();

        // 非受控区注释保留（[tui] 表头前 + 表内 text 前——片段只在 tui 补子键、
        // 新增 mcp_servers，不重写这些行）。
        assert!(
            written.contains("# 共享偏好"),
            "非受控表头注释保留: {written}"
        );
        assert!(written.contains("# 主题"), "非受控表内注释保留: {written}");
        // 身份键受控替换（old→kimi）；非受控 [tui].text live 赢；片段独有键补上。
        assert_eq!(
            get_str(&written, &["model"]).as_deref(),
            Some("kimi-k2.7-code")
        );
        assert_eq!(
            get_str(&written, &["tui", "text"]).as_deref(),
            Some("light")
        );
        assert_eq!(
            get_str(&written, &["tui", "top_output_style"]).as_deref(),
            Some("compact")
        );
        assert_eq!(
            get_str(&written, &["mcp_servers", "github", "command"]).as_deref(),
            Some("npx")
        );
        // 键序保留：model（顶层）→ [tui] → 片段补的 [mcp_servers.github] 依次在后。
        let model_pos = written.find("model =").unwrap();
        let tui_pos = written.find("[tui]").unwrap();
        let mcp_pos = written.find("[mcp_servers.github]").unwrap();
        assert!(model_pos < tui_pos, "顶层键序保留：model 在 [tui] 之前");
        assert!(tui_pos < mcp_pos, "片段补的表追加在 live 既有表之后");
    }
}
