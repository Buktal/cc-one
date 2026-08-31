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

use crate::error::AppResult;
use crate::model::App;
use crate::provider::settings_codec::parse_grok_settings;

/// profile 块表名：`[model.<profile>]`。注意与 [`MODELS_TABLE`]（default 指针
/// 表）是两个不同的顶层表，别混淆。`pub(crate)`：写盘（本模块）与导入反向
/// 解析（`import_live` 读 cc-one profile、判可共享键）共用——改表名只改这里，
/// 不许在调用方裸写字面量。
pub(crate) const MODEL_TABLE: &str = "model";
/// default 指针表名：`[models]` 的 `default = "<profile>"`。可见性理由同
/// [`MODEL_TABLE`]（导入反向解析判「可共享键」时排除本表）。
pub(crate) const MODELS_TABLE: &str = "models";
/// cc one 固定持有的 canonical profile 名（`cc-one` 是合法 TOML bare key）。
/// 用固定名而非供应商名，避免改名 / 重命名脆弱性；用户自己的 profile 用别的
/// 名字，互不冲突。可见性理由同 [`MODEL_TABLE`]（导入反向解析按它定位
/// cc-one profile）。
pub(crate) const CC_ONE_PROFILE: &str = "cc-one";

/// `~/.grok` 目录（跨平台统一走 home；家目录映射归
/// [`App::app_config_dir`]，单一声明处）。
pub fn grok_config_dir() -> AppResult<PathBuf> {
    App::Grok.app_config_dir()
}

/// `~/.grok/config.toml` 路径。
pub fn grok_config_path() -> AppResult<PathBuf> {
    Ok(grok_config_dir()?.join("config.toml"))
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
    let mut doc = crate::provider::live::parse_toml_or_empty(live, "live config.toml")?;
    let target_doc = crate::provider::live::parse_toml_or_empty(target, "provider config.toml")?;

    // 目标的 cc-one profile 块（Option）。有 → 激活供应商；无 → 登录态版。
    let target_profile = target_doc
        .get(MODEL_TABLE)
        .and_then(|t| t.get(CC_ONE_PROFILE))
        .cloned();

    match target_profile {
        Some(profile) => {
            // 受控写入：替换 [model."cc-one"] + 把 models.default 指向它。model 表
            // 只装 profile 子表、从不持有直接键值，标为隐式——只渲染
            // [model."cc-one"]、不产出孤立的 [model] 头（toml_edit 对显式空父表与
            // 其它顶层表共存时渲染不稳，曾致两连切键序漂移、不幂等）。
            let model = table_mut(&mut doc, MODEL_TABLE);
            model.insert(CC_ONE_PROFILE, profile);
            model.set_implicit(true);
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

/// 写盘层片段补缺失纯函数：在 `merge_grok_config` 产出的合并结果上，把片段的
/// 非受控键补进去（live 已有则保留、递归进子表）。身份键（`[model."cc-one"]`
/// profile 块 + `models.default` 指针）归 cc one，片段不得携带 → `Err`（与
/// [`validate_grok_snippet`] 同款拒绝，set 命令提前拦 + 合并纯函数兜底）。为什么
/// 在写盘层：同 codex（见 ADR-0010）。`mcp_servers` 含凭据允许（不经 LLM 端点）。
///
/// 边界：`snippet` 空串/纯空白 → 空文档（无操作）；非空非法 TOML → `Err`；
/// 含身份键 → `Err`。
pub fn merge_grok_snippet(merged: &str, snippet: &str) -> AppResult<String> {
    crate::provider::live::merge_toml_snippet(merged, snippet, "grok", grok_identity_hit)
}

/// grok 片段校验（set 命令用）：合法 TOML；不得含受控身份键。凭据键不禁
/// （grok 片段写 `mcp_servers` 等、不经 LLM 端点，见 ADR-0010）。骨架与
/// codex 共用（`live::validate_toml_snippet`），只有身份键谓词是 grok 自己的。
pub fn validate_grok_snippet(snippet: &str) -> AppResult<()> {
    crate::provider::live::validate_toml_snippet(snippet, "grok", grok_identity_hit)
}

/// grok 身份键命中描述（报错用，#55）：复用 [`snippet_identity_hit`] 的判定
/// （cc-one profile 块 / default 指针），包成共用骨架要的 `Option<String>`。
fn grok_identity_hit(doc: &DocumentMut) -> Option<String> {
    snippet_identity_hit(doc).map(str::to_string)
}

/// 片段携带哪个 grok 受控身份键：`[model."cc-one"]` profile 块 或
/// `models.default` 指针——返回命中的那个（报错指明具体键，#55：键名细节
/// 只出现在校验报错里）。用户自建的 `[model.<其它>]` profile 不是身份键
/// （允许进片段）。
fn snippet_identity_hit(doc: &DocumentMut) -> Option<&'static str> {
    if doc
        .get(MODEL_TABLE)
        .and_then(|t| t.get(CC_ONE_PROFILE))
        .is_some()
    {
        return Some("[model.\"cc-one\"] 块");
    }
    if doc
        .get(MODELS_TABLE)
        .and_then(|t| t.get("default"))
        .is_some()
    {
        return Some("models.default");
    }
    None
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

/// 切换写盘全流程（薄壳，按序调用）：解析快照 → TOML 受控合并 → 写盘层补片段
/// → 无变化则无操作 → 备份 → 原子写（事务收口在 `live::commit_live_file`，
/// 五个 app 共用）。
///
/// `snippet` 是写盘层通用片段（与 codex 同构，见 ADR-0010）：`merge_grok_config`
/// 只搬受控身份键、丢弃 target 其余键，故片段必须在 merge 之后补进 live doc
/// （settings_config 层合会被写盘白名单滤掉→片段零效果）。空串即无操作。
pub fn switch_grok_live(config_path: &Path, settings_config: &str, snippet: &str) -> AppResult<()> {
    let target_toml = parse_grok_settings(settings_config)?;
    let live = crate::provider::live::read_live_settings(config_path)?;
    let mut merged = merge_grok_config(&live, &target_toml)?;
    // 写盘层补片段：merge_grok_config 只搬受控身份键、丢弃其余，故片段必须在此补
    // （settings_config 层合会被白名单滤掉→零效果，见 ADR-0010）。片段空则跳过。
    if !snippet.trim().is_empty() {
        merged = merge_grok_snippet(&merged, snippet)?;
    }
    let unchanged = crate::provider::live::content_unchanged(&live, &merged);
    crate::provider::live::commit_live_file(config_path, &merged, unchanged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::provider::testutil;
    use std::fs;

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

    #[test]
    fn controlled_profile_replaced_uncontrolled_preserved() {
        let live = testutil::live_with_uncontrolled(App::Grok);
        let target = third_party_target("grok-4.5", "https://api.x.ai/v1", "xAI (Grok)");
        let merged = merge_grok_config(&live, &target).unwrap();
        // cc-one profile 写入 + default 指向它。
        assert_eq!(
            testutil::toml_get_str(&merged, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            testutil::toml_get_str(&merged, &["model", "cc-one", "base_url"]).as_deref(),
            Some("https://api.x.ai/v1")
        );
        assert_eq!(
            testutil::toml_get_str(&merged, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        // 用户的 my-custom profile 原样保留。
        assert_eq!(
            testutil::toml_get_str(&merged, &["model", "my-custom", "model"]).as_deref(),
            Some("grok-3"),
            "用户自建 profile 必须保留"
        );
        // mcp_servers 原样保留。
        assert_eq!(
            testutil::toml_get_str(&merged, &["mcp_servers", "filesystem", "command"]).as_deref(),
            Some("npx")
        );
    }

    #[test]
    fn target_non_cc_one_profiles_are_ignored() {
        let live = testutil::live_with_uncontrolled(App::Grok);
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
            testutil::toml_get_str(&merged, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert!(
            testutil::toml_get_str(&merged, &["model", "other"]).is_none(),
            "目标的非 cc-one profile 不得写入"
        );
    }

    #[test]
    fn empty_live_merges_to_cc_one_only() {
        let target = third_party_target("grok-4.5", "https://api.x.ai/v1", "xAI");
        let merged = merge_grok_config("", &target).unwrap();
        assert_eq!(
            testutil::toml_get_str(&merged, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            testutil::toml_get_str(&merged, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        assert!(
            testutil::toml_get_str(&merged, &["mcp_servers"]).is_none(),
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
            testutil::toml_get_str(&merged, &["model", "cc-one"]).is_none(),
            "登录态版必须移除 cc-one profile"
        );
        assert!(
            testutil::toml_get_str(&merged, &["models", "default"]).is_none(),
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
            testutil::toml_get_str(&merged, &["models", "default"]).as_deref(),
            Some("my-custom"),
            "用户自设的 default 不得被登录态切换清掉"
        );
        assert_eq!(
            testutil::toml_get_str(&merged, &["model", "my-custom", "model"]).as_deref(),
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
        let config_path = seed(
            tmp.path(),
            Some(&testutil::live_with_uncontrolled(App::Grok)),
        );

        switch_grok_live(
            &config_path,
            r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"k\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\""}"#,
            "",
        )
        .unwrap();

        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            testutil::toml_get_str(&written, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            testutil::toml_get_str(&written, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        assert_eq!(
            testutil::toml_get_str(&written, &["model", "my-custom", "model"]).as_deref(),
            Some("grok-3"),
            "用户 profile 保留"
        );
        assert_eq!(
            testutil::toml_get_str(&written, &["mcp_servers", "filesystem", "command"]).as_deref(),
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
        switch_grok_live(&config_path, r#"{"config":""}"#, "").unwrap();

        let written = fs::read_to_string(&config_path).unwrap();
        assert!(testutil::toml_get_str(&written, &["model", "cc-one"]).is_none());
        assert!(
            testutil::toml_get_str(&written, &["models", "default"]).is_none(),
            "我们设的 default 被清掉"
        );
        assert_eq!(
            testutil::toml_get_str(&written, &["mcp_servers", "filesystem", "command"]).as_deref(),
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
            "",
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
            "",
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
            "",
        )
        .unwrap();
        assert!(config_path.exists());
        assert_eq!(
            testutil::toml_get_str(
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
        switch_grok_live(&config_path, target, "").unwrap();
        let written = fs::read_to_string(&config_path).unwrap();
        assert_eq!(
            testutil::toml_get_str(&written, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        let bak = tmp.path().join("config.toml.bak");
        assert!(bak.exists(), "首次写入（live 已存在）应备份");
        fs::remove_file(&bak).unwrap();
        switch_grok_live(&config_path, target, "").unwrap();
        assert_eq!(fs::read_to_string(&config_path).unwrap(), written);
        assert!(!bak.exists(), "全无变化 → 不备份");
    }

    #[test]
    fn switch_writes_snippet_mcp_servers() {
        // 切换时片段的非受控键（mcp_servers）补进 live doc（写盘层，ADR-0010）。
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(
            tmp.path(),
            Some("[models]\ndefault = \"old\"\n\n[model.old]\nmodel = \"grok-3\"\n"),
        );
        let target = r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\""}"#;
        let snippet = "[mcp_servers.github]\ncommand = \"npx\"\n";

        switch_grok_live(&config_path, target, snippet).unwrap();

        let written = fs::read_to_string(&config_path).unwrap();
        // 受控：cc-one profile + default 指针。
        assert_eq!(
            testutil::toml_get_str(&written, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            testutil::toml_get_str(&written, &["models", "default"]).as_deref(),
            Some("cc-one")
        );
        // 片段补的 mcp_servers 落盘（含凭据也允许——不经 LLM 端点）。
        assert_eq!(
            testutil::toml_get_str(&written, &["mcp_servers", "github", "command"]).as_deref(),
            Some("npx"),
            "片段的 mcp_servers 经切换写盘补进 live"
        );
    }

    #[test]
    fn switch_rejects_snippet_with_identity_key() {
        // 片段含身份键（cc-one profile / models.default）→ 合并拒绝，切换失败、
        // 不写盘（与 validate_grok_snippet 同款，防绕过 set 命令的路径）。
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some("[models]\ndefault = \"old\"\n"));
        let target = r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\""}"#;

        let r = switch_grok_live(&config_path, target, "[model.cc-one]\nmodel = \"x\"\n");
        assert!(r.is_err(), "片段含身份键必须拒绝切换");

        // 拒绝路径不写盘：live 原样。
        assert_eq!(
            testutil::toml_get_str(
                &fs::read_to_string(&config_path).unwrap(),
                &["models", "default"]
            )
            .as_deref(),
            Some("old"),
            "拒绝路径不得写盘"
        );
    }

    #[test]
    fn switch_with_snippet_is_idempotent() {
        // 同一供应商连切多次（含片段）：稳态字节不变、不重复备份；身份键与片段键
        // 各只出现一次（fill_missing 只补缺失，不重复追加——ADR-0010）。
        //
        // 注：toml_edit 对**隐式身份表 model**（[model.cc-one] 的隐式父表）与片段
        // 新加的**显式顶层表**（[mcp_servers]）的兄弟顺序，在 parse/modify 循环里
        // 非确定——首轮 merge 新建 model（隐式）后 fill_missing 插 mcp_servers，
        // toml_edit 把显式表摆在隐式表前；第二轮 merge 重触 model 又把它归位到前。
        // 故**首次切换会做一次性兄弟序归一**（只动顺序、不改语义、不增键），自第
        // 二次起字节幂等。强求首轮也字节幂等，要么规范化表序（破坏「键序保留」
        // 验收）、要么缠斗 toml_edit 隐式表内部（DocumentMut 不实现 PartialEq，语
        // 义比对需另引 toml crate），代价超过这个良性边界场景的收益。
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some("[models]\ndefault = \"old\"\n"));
        let target = r#"{"config":"[model.cc-one]\nmodel = \"grok-4.5\""}"#;
        let snippet = "[mcp_servers.github]\ncommand = \"npx\"\n";

        // 三连切；前两次含首轮归一，第三次起进入稳态。
        switch_grok_live(&config_path, target, snippet).unwrap();
        switch_grok_live(&config_path, target, snippet).unwrap();
        let steady = fs::read_to_string(&config_path).unwrap();
        let bak = tmp.path().join("config.toml.bak");
        fs::remove_file(&bak).unwrap();
        switch_grok_live(&config_path, target, snippet).unwrap();

        // 稳态：第三次与第二次字节相同、不重复备份。
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            steady,
            "稳态字节幂等"
        );
        assert!(!bak.exists(), "稳态无变化 → 不重复备份");
        // 不重复追加（最关键不变量）：身份键 cc-one 与片段键 mcp_servers 各只一次。
        assert_eq!(
            steady.matches("[model.cc-one]").count(),
            1,
            "cc-one 不重复追加"
        );
        assert_eq!(
            steady.matches("[mcp_servers.github]").count(),
            1,
            "mcp_servers 不重复追加"
        );
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

    #[test]
    fn snippet_fills_missing_mcp_servers() {
        // merged（merge_grok_config 产物，含 cc-one profile）没有 mcp_servers，
        // 片段补上。
        let merged = r#"[model.cc-one]
model = "grok-4.5"
"#;
        let snippet = r#"[mcp_servers.github]
command = "npx"
"#;
        let out = merge_grok_snippet(merged, snippet).unwrap();
        assert_eq!(
            testutil::toml_get_str(&out, &["model", "cc-one", "model"]).as_deref(),
            Some("grok-4.5")
        );
        assert_eq!(
            testutil::toml_get_str(&out, &["mcp_servers", "github", "command"]).as_deref(),
            Some("npx")
        );
    }

    #[test]
    fn snippet_live_wins_on_shared_mcp_server() {
        let merged = r#"[mcp_servers.shared]
command = "live"
"#;
        let snippet = r#"[mcp_servers.shared]
command = "snippet"
extra = "from-snippet"
"#;
        let out = merge_grok_snippet(merged, snippet).unwrap();
        assert_eq!(
            testutil::toml_get_str(&out, &["mcp_servers", "shared", "command"]).as_deref(),
            Some("live"),
            "同 server 键 live 已有 → 不覆盖"
        );
        assert_eq!(
            testutil::toml_get_str(&out, &["mcp_servers", "shared", "extra"]).as_deref(),
            Some("from-snippet"),
            "递归补缺失：片段独有子键补上"
        );
    }

    #[test]
    fn snippet_with_identity_key_is_rejected() {
        // cc-one profile 块 / models.default 是身份键 → 拒。
        assert!(merge_grok_snippet("", "[model.cc-one]\nmodel = \"x\"\n").is_err());
        assert!(merge_grok_snippet("", "[models]\ndefault = \"cc-one\"\n").is_err());
        // 用户自建 profile（非 cc-one）不是身份键 → 允许。
        assert!(merge_grok_snippet("", "[model.my-custom]\nmodel = \"x\"\n").is_ok());
    }

    #[test]
    fn empty_snippet_is_noop_for_grok_merge() {
        let merged = "[model.cc-one]\nmodel = \"grok-4.5\"\n";
        for empty in ["", "   ", "\n"] {
            let out = merge_grok_snippet(merged, empty).unwrap();
            assert_eq!(
                testutil::toml_get_str(&out, &["model", "cc-one", "model"]).as_deref(),
                Some("grok-4.5")
            );
        }
    }

    #[test]
    fn invalid_grok_snippet_toml_is_error() {
        assert!(merge_grok_snippet("[model.cc-one]\nmodel=\"m\"", "not toml {").is_err());
    }

    #[test]
    fn grok_snippet_fill_missing_is_idempotent() {
        let merged = "[model.cc-one]\nmodel = \"grok-4.5\"\n";
        let snippet = "[mcp_servers.github]\ncommand = \"npx\"\n";
        let once = merge_grok_snippet(merged, snippet).unwrap();
        let twice = merge_grok_snippet(&once, snippet).unwrap();
        assert_eq!(once, twice);
        assert_eq!(
            testutil::toml_get_str(&twice, &["mcp_servers", "github", "command"]).as_deref(),
            Some("npx")
        );
    }

    #[test]
    fn validate_grok_snippet_accepts_shared_and_rejects_identity() {
        // 合法：mcp_servers（含凭据）+ 用户自建 profile。
        assert!(validate_grok_snippet(
            "[mcp_servers.github]\ncommand = \"npx\"\nenv = { GITHUB_PERSONAL_ACCESS_TOKEN = \"ghp_x\" }\n"
        )
        .is_ok());
        assert!(validate_grok_snippet("[model.my-custom]\nmodel = \"x\"\n").is_ok());
        assert!(validate_grok_snippet("").is_ok());
        // 拒绝：身份键。
        assert!(validate_grok_snippet("[model.cc-one]\nmodel = \"x\"\n").is_err());
        assert!(validate_grok_snippet("[models]\ndefault = \"cc-one\"\n").is_err());
        assert!(validate_grok_snippet("not toml {").is_err());
    }
}
