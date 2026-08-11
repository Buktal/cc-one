//! Grok 写盘（live_grok）：TOML 受控合并，单文件 `~/.grok/config.toml`。
//!
//! 与 claude / codex / gemini 同一套「受控合并 / 非受控保留 / 备份 / 原子写」
//! 原则，但 Grok 的配置结构是「命名 profile」式 TOML：
//! - 目标文件：`~/.grok/config.toml`（**单文件，无 auth.json**——api_key 内联
//!   在 profile 块里，比 codex 两文件更简）。
//! - 结构：`[models]` 表的 `default = "<profile>"` 指向当前激活 profile；
//!   `[model.<profile>]` 是各供应商块（`model` / `base_url` / `api_key` /
//!   `env_key` / `api_backend` / `context_window` / `name`）。
//! - **受控字段**：cc one 固定拥有一个 canonical profile `[model."cc-one"]` +
//!   `models.default` 指针。切换时整块替换该 profile 块、把 default 指向它；
//!   用户手动的其它 `[model.*]` profile、`[mcp_servers]` 等非受控字段从 live
//!   原样保留（`toml_edit` 重写保留注释与格式）。**绝不整文件覆盖**。
//! - **登录态版**（官方预设，settingsConfig 空 / 无 cc-one profile）：撤掉
//!   cc-one 的足迹——移除 `[model."cc-one"]`，且仅当 `models.default` 仍指向
//!   cc-one 时清掉它（用户自设的 default 不动），让 Grok CLI 回落自带 xAI
//!   OAuth 订阅登录。
//! - 写前备份 `config.toml.bak`（单份覆盖）；原子写（临时文件 + 改名）。
//! - 清洗：写盘前剥 settingsConfig 里的内部 meta 字段（沿用
//!   `live::LIVE_INTERNAL_KEYS` 语义）。
//!
//! `merge_grok_config` 是纯函数（本项目最高价值的测试接缝之一）：输入
//! (当前 live TOML 文本, 目标 TOML 文本) → 输出合并后的 TOML 文本，不碰
//! 文件系统。「cc-one profile 替换 / 用户其它 profile 保留」这个关键不变量
//! 靠它落进可测代码。

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table};

use crate::error::{AppError, AppResult};

/// profile 块表名：`[model.<profile>]`。注意与 [`MODELS_TABLE`]（default 指针
/// 表）是两个不同的顶层表，别混淆。
const MODEL_TABLE: &str = "model";
/// default 指针表名：`[models]` 的 `default = "<profile>"`。
const MODELS_TABLE: &str = "models";
/// cc one 固定持有的 canonical profile 名（`cc-one` 是合法 TOML bare key）。
/// 用固定名而非供应商名，避免改名 / 重命名脆弱性；用户自己的 profile 用别的
/// 名字，互不冲突。
const CC_ONE_PROFILE: &str = "cc-one";

/// `~/.grok` 目录（跨平台统一走 home）。
pub fn grok_config_dir() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".grok"))
}

/// `~/.grok/config.toml` 路径。
pub fn grok_config_path() -> AppResult<PathBuf> {
    Ok(grok_config_dir()?.join("config.toml"))
}

/// 解析供应商 settingsConfig（`{"config": "<TOML>"}` JSON 对象）为写盘目标
/// TOML 文本：剥内部 meta 字段（沿用 `LIVE_INTERNAL_KEYS` 语义），提取
/// `config`。
///
/// 边界：空串/纯空白 → 空目标（登录态版）；非对象 settingsConfig、非字符串
/// `config` → `Err`（坏配置不能进用户 config.toml）。
pub fn parse_grok_settings(settings_config: &str) -> AppResult<String> {
    let trimmed = settings_config.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let mut obj = crate::provider::live::parse_object(trimmed, "provider settingsConfig")?;
    // 清洗内部 meta 字段：这些键只供应用自己读，不是 config.toml 的合法字段
    // （与 claude / codex 分支同一份清单、同一套语义）。
    if let Some(o) = obj.as_object_mut() {
        for key in crate::provider::live::LIVE_INTERNAL_KEYS {
            o.remove(*key);
        }
    }
    match obj.get("config") {
        None => Ok(String::new()),
        Some(v) => v.as_str().map(str::to_string).ok_or_else(|| {
            AppError::Config("provider settingsConfig config must be a TOML string".into())
        }),
    }
}

/// TOML 受控合并纯函数（最高价值测试接缝）：目标（供应商快照）里出现的
/// `[model."cc-one"]` profile 块整块替换进 live + `models.default` 指向它；
/// 用户其它 `[model.*]` profile、`[mcp_servers]` 等非受控字段从 live 原样保留
/// （`toml_edit` 重写保留注释与格式）。不碰文件系统。
///
/// 登录态版（目标无 cc-one profile）：移除 live 的 `[model."cc-one"]`，且仅当
/// `models.default` 仍指向 cc-one 时清掉它——用户自设的 default 不动。
///
/// 边界：live / target 为空串或纯空白 → 视为空文档；非空非法 TOML → `Err`
/// （live 解析不了就没法保留用户手动配置、target 解析不了不能进用户
/// config.toml，都宁可失败）。
pub fn merge_grok_config(live: &str, target: &str) -> AppResult<String> {
    let mut doc = parse_toml_or_empty(live, "live config.toml")?;
    let target_doc = parse_toml_or_empty(target, "provider config.toml")?;

    // 目标的 cc-one profile 块（Option）。有 → 激活供应商；无 → 登录态版。
    let target_profile = target_doc
        .get(MODEL_TABLE)
        .and_then(|t| t.get(CC_ONE_PROFILE))
        .cloned();

    match target_profile {
        Some(profile) => {
            // 受控写入：替换 [model."cc-one"] + 把 models.default 指向它。
            table_mut(&mut doc, MODEL_TABLE).insert(CC_ONE_PROFILE, profile);
            table_mut(&mut doc, MODELS_TABLE).insert("default", toml_edit::value(CC_ONE_PROFILE));
        }
        None => {
            // 登录态版：撤掉 cc-one 足迹。只清掉我们自己设的 default 指针。
            if let Some(tbl) = doc.get_mut(MODEL_TABLE).and_then(|i| i.as_table_mut()) {
                tbl.remove(CC_ONE_PROFILE);
            }
            let ours = doc
                .get(MODELS_TABLE)
                .and_then(|t| t.get("default"))
                .and_then(|d| d.as_str())
                .is_some_and(|s| s == CC_ONE_PROFILE);
            if ours {
                if let Some(tbl) = doc.get_mut(MODELS_TABLE).and_then(|i| i.as_table_mut()) {
                    tbl.remove("default");
                }
            }
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

/// 取 doc 里某顶层表的可变引用；不存在（或不是表）则替换为空表后返回。
/// 非 table 项（如用户写坏 `model = "x"`）被空表顶替——那本就不是合法 grok
/// 配置，顶替比 panic 更稳。
fn table_mut<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    if !doc.get(key).is_some_and(|i| i.is_table()) {
        doc.insert(key, Item::Table(Table::new()));
    }
    doc.get_mut(key)
        .expect("just inserted")
        .as_table_mut()
        .expect("just ensured table")
}

/// 切换写盘全流程（薄壳，按序调用）：解析快照 → TOML 受控合并 → 无变化则
/// 无操作 → 备份 → 原子写。
pub fn switch_grok_live(config_path: &Path, settings_config: &str) -> AppResult<()> {
    let target_toml = parse_grok_settings(settings_config)?;
    let live = crate::provider::live::read_live_settings(config_path)?;
    let merged = merge_grok_config(&live, &target_toml)?;

    // 内容无变化 → 无操作（不备份、不写盘、不碰 mtime）。trim_end 容忍
    // toml_edit 重写时对结尾换行的归一化。
    if merged.trim_end() == live.trim_end() {
        return Ok(());
    }
    crate::provider::live::backup_file(config_path)
        .and_then(|()| crate::provider::live::atomic_write_file(config_path, &merged))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 一份带用户手动配置（非受控字段）的 live config.toml：用户自建 profile
    /// + mcp_servers 都要原样保留。
    fn live_with_uncontrolled() -> String {
        r#"# 用户手动的配置
[models]
default = "my-custom"

[model.my-custom]
model = "grok-3"
base_url = "https://old.dev"
api_key = "old-key"
api_backend = "responses"
context_window = 100000
name = "My Custom"

[mcp_servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
"#
        .to_string()
    }

    /// 目标快照 TOML（第三方预设形状：cc-one profile 块）。
    fn third_party_target(model: &str, base_url: &str, name: &str) -> String {
        format!(
            r#"[model.cc-one]
model = {model:?}
base_url = {base_url:?}
api_key = "user-key"
api_backend = "responses"
context_window = 500000
name = {name:?}
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
            .or_else(|| cur.as_integer().map(|i| i.to_string()))
    }

    #[test]
    fn controlled_profile_replaced_uncontrolled_preserved() {
        let live = live_with_uncontrolled();
        let target = third_party_target("grok-4.5", "https://api.x.ai/v1", "xAI (Grok)");
        let merged = merge_grok_config(&live, &target).unwrap();
        // cc-one profile 写入 + default 指向它。
        assert_eq!(
            get_str(&merged, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            get_str(&merged, &["model", "cc-one", "base_url"]).as_deref(),
            Some("https://api.x.ai/v1")
        );
        assert_eq!(
            get_str(&merged, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        // 用户的 my-custom profile 原样保留。
        assert_eq!(
            get_str(&merged, &["model", "my-custom", "model"]).as_deref(),
            Some("grok-3"),
            "用户自建 profile 必须保留"
        );
        // mcp_servers 原样保留。
        assert_eq!(
            get_str(&merged, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx")
        );
    }

    #[test]
    fn target_non_cc_one_profiles_are_ignored() {
        let live = live_with_uncontrolled();
        // 目标带了 cc-one 之外的 profile（target 里的 [model.other]）——非受控，
        // 绝不能写进 live。
        let target = r#"[model.cc-one]
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
api_key = "k"
api_backend = "responses"
context_window = 500000
name = "xAI"

[model.other]
model = "sneaky"
base_url = "https://evil.dev"
"#;
        let merged = merge_grok_config(&live, target).unwrap();
        assert_eq!(
            get_str(&merged, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert!(
            get_str(&merged, &["model", "other"]).is_none(),
            "目标的非 cc-one profile 不得写入"
        );
    }

    #[test]
    fn empty_live_merges_to_cc_one_only() {
        let target = third_party_target("grok-4.5", "https://api.x.ai/v1", "xAI");
        let merged = merge_grok_config("", &target).unwrap();
        assert_eq!(
            get_str(&merged, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            get_str(&merged, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        assert!(
            get_str(&merged, &["mcp_servers"]).is_none(),
            "live 为空时目标里也不得引入非受控字段"
        );
    }

    #[test]
    fn empty_target_is_login_state_removes_cc_one_footprint() {
        // live 里有 cc-one（我们之前激活的）+ default 指向它。
        let live = r#"[models]
default = "cc-one"

[model.cc-one]
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
"#;
        let merged = merge_grok_config(live, "").unwrap();
        assert!(
            get_str(&merged, &["model", "cc-one"]).is_none(),
            "登录态版必须移除 cc-one profile"
        );
        assert!(
            get_str(&merged, &["models", "default"]).is_none(),
            "登录态版必须清掉我们设的 default 指针"
        );
    }

    #[test]
    fn login_state_preserves_user_set_default() {
        // 用户把 default 指向自己的 profile（不是 cc-one）→ 登录态切换不得动它。
        let live = r#"[models]
default = "my-custom"

[model.my-custom]
model = "grok-3"
"#;
        let merged = merge_grok_config(live, "").unwrap();
        assert_eq!(
            get_str(&merged, &["models", "default"]).as_deref(),
            Some("my-custom"),
            "用户自设的 default 不得被登录态切换清掉"
        );
        assert_eq!(
            get_str(&merged, &["model", "my-custom", "model"]).as_deref(),
            Some("grok-3")
        );
    }

    #[test]
    fn comment_and_format_preserved_on_untouched_lines() {
        let live = r#"# 用户手动的配置
[models]
default = "my-custom"

[model.my-custom]
model   =   "grok-3"
"#;
        // 目标只动 cc-one，不碰 my-custom——my-custom 行要逐字节保留。
        let target = third_party_target("grok-4.5", "https://api.x.ai/v1", "xAI");
        let merged = merge_grok_config(live, &target).unwrap();
        assert!(
            merged.contains("model   =   \"grok-3\""),
            "未受控行的格式必须逐字节保留: {merged}"
        );
        assert!(
            merged.contains("# 用户手动的配置"),
            "注释必须保留: {merged}"
        );
    }

    #[test]
    fn invalid_live_toml_is_an_error() {
        let r = merge_grok_config("model = [1,2", &third_party_target("m", "u", "n"));
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "live 非法 TOML 必须失败——解析不了就没法保留用户手动配置"
        );
    }

    #[test]
    fn invalid_target_toml_is_an_error() {
        let r = merge_grok_config("", "not toml {");
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "目标非法 TOML 必须失败——坏配置不能进用户 config.toml"
        );
    }

    #[test]
    fn parse_settings_extracts_config() {
        let s = parse_grok_settings(r#"{"config":"[model.cc-one]\nmodel = \"m\""}"#).unwrap();
        assert!(s.contains("[model.cc-one]"));
        assert!(s.contains(r#"model = "m""#));
    }

    #[test]
    fn parse_settings_strips_internal_meta_keys() {
        let s = parse_grok_settings(
            r#"{"api_format":"openai","apiFormat":"openai","openrouter_compat_mode":true,"config":"[model.cc-one]\nmodel = \"m\""}"#,
        )
        .unwrap();
        assert!(s.contains("[model.cc-one]"));
    }

    #[test]
    fn parse_settings_rejects_bad_shapes() {
        assert!(parse_grok_settings("[1,2]").is_err());
        assert!(parse_grok_settings(r#""just a string""#).is_err());
        assert!(parse_grok_settings(r#"{"config":123}"#).is_err());
    }

    #[test]
    fn empty_settings_is_empty_target() {
        for raw in ["", "   "] {
            assert_eq!(parse_grok_settings(raw).unwrap(), "");
        }
        // 无 config 字段也视为空目标。
        assert_eq!(parse_grok_settings("{}").unwrap(), "");
    }

    /// 临时目录里放好 config.toml（模拟用户 live 配置）。
    fn seed(tmp: &Path, config: Option<&str>) -> PathBuf {
        let config_path = tmp.join("config.toml");
        if let Some(c) = config {
            fs::write(&config_path, c).unwrap();
        }
        config_path
    }

    #[test]
    fn switch_writes_profile_and_preserves_user_blocks() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some(&live_with_uncontrolled()));

        switch_grok_live(
            &config_path,
            r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"k\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\""}"#,
        )
        .unwrap();

        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            get_str(&written, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            get_str(&written, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        assert_eq!(
            get_str(&written, &["model", "my-custom", "model"]).as_deref(),
            Some("grok-3"),
            "用户 profile 保留"
        );
        assert_eq!(
            get_str(&written, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx"),
            "mcp_servers 保留"
        );
    }

    #[test]
    fn login_state_switch_removes_cc_one_keeps_user_config() {
        let tmp = tempfile::tempdir().unwrap();
        // 先激活 cc-one。
        let config_path = seed(
            tmp.path(),
            Some(
                r#"[models]
default = "cc-one"

[model.cc-one]
model = "grok-4.5"
base_url = "https://api.x.ai/v1"

[mcp_servers.filesystem]
command = "npx"
"#,
            ),
        );

        // 切到登录态版（空快照）。
        switch_grok_live(&config_path, r#"{"config":""}"#).unwrap();

        let written = fs::read_to_string(&config_path).unwrap();
        assert!(get_str(&written, &["model", "cc-one"]).is_none());
        assert!(
            get_str(&written, &["models", "default"]).is_none(),
            "我们设的 default 被清掉"
        );
        assert_eq!(
            get_str(&written, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx"),
            "用户的 mcp_servers 保留"
        );
    }

    #[test]
    fn backup_created_when_changes_and_not_when_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(
            tmp.path(),
            Some("[models]\ndefault = \"old\"\n\n[model.old]\nmodel = \"grok-3\"\n"),
        );

        switch_grok_live(
            &config_path,
            r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"k\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\""}"#,
        )
        .unwrap();

        let bak = tmp.path().join("config.toml.bak");
        assert!(bak.exists(), "config 变化必须备份");
        assert!(
            fs::read_to_string(&bak).unwrap().contains("grok-3"),
            ".bak 是写盘前的 live 快照"
        );

        // 再次切到同一内容 → 无操作、无新备份。
        let before = fs::read_to_string(&config_path).unwrap();
        fs::remove_file(&bak).unwrap();
        switch_grok_live(
            &config_path,
            r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"k\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\""}"#,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), before);
        assert!(!bak.exists(), "内容无变化不得触发备份");
    }

    #[test]
    fn config_missing_creates_file_without_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), None);
        switch_grok_live(
            &config_path,
            r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"k\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\""}"#,
        )
        .unwrap();
        assert!(config_path.exists());
        assert_eq!(
            get_str(
                &fs::read_to_string(&config_path).unwrap(),
                &["model", "cc-one", "model"]
            )
            .as_deref(),
            Some("grok-4.5")
        );
        assert!(
            !tmp.path().join("config.toml.bak").exists(),
            "live 原本不存在 → 无备份"
        );
    }

    #[test]
    fn switch_is_noop_when_nothing_changes() {
        let tmp = tempfile::tempdir().unwrap();
        // seed 一份现有 live（旧 profile）——首次切换会真改并备份。
        let config_path = seed(
            tmp.path(),
            Some("[models]\ndefault = \"old\"\n\n[model.old]\nmodel = \"grok-3\"\n"),
        );
        // 第一次切换写出 merged；同样的快照再切一次 → 无操作。用「同一输入
        // 两连切」保证判定不依赖 toml_edit 的字节渲染细节。
        let target = r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"k\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\""}"#;
        switch_grok_live(&config_path, target).unwrap();
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            get_str(&written, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        let bak = tmp.path().join("config.toml.bak");
        assert!(bak.exists(), "首次写入（live 已存在）应备份");
        fs::remove_file(&bak).unwrap();
        switch_grok_live(&config_path, target).unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), written);
        assert!(!bak.exists(), "全无变化 → 不备份");
    }

    #[test]
    fn grok_paths_point_at_home_grok_dir() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(grok_config_dir().unwrap(), home.join(".grok"));
        assert_eq!(
            grok_config_path().unwrap(),
            home.join(".grok").join("config.toml")
        );
    }
}
