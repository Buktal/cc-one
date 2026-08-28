//! Per-app live 行为 adapter（单一 seam）：所有「app X 的 live 文件长什么样、
//! 受控字段是哪些、片段在哪层合并、模型怎么拉」的答案收在本模块一个 `impl App`
//! 里——「按 App 分派」只发生在这一处，其它调用方一律向 `app` 询问，不再自己
//! match。
//!
//! 收敛前这些知识散在 8 处平行 `match App`（write_live / read_live_texts /
//! read_live_snapshot / validate_snippet / merge_extracted_snippet / switch 片段
//! 层策略 ×2 / fetch_models 协议），没有 owner：加第 6 个 app 要同时改 8 处。
//! 现在每个方法都是对 `App` 的穷尽 match——加 app 时编译器会在每个方法里逼出
//! 新分支，不再有「漏改一处」的静默漂移。
//!
//! 五个 app 的行为一览（各方法的分支是唯一声明处，下表是给读者的导航）：
//!
//! | app | live 文件（[`App::live_paths`]） | 写盘（[`App::write_live`]） | 导入条目（[`App::live_import_entries`]） | 片段提取（[`App::extract_snippet`]） | 片段校验（[`App::validate_snippet`]） | 片段合并层与域（[`App::snippet_layer`]，ADR-0010） | 模型协议（[`App::fetch_models`]） |
//! | --- | --- | --- | --- | --- | --- | --- | --- |
//! | claude | `settings.json` | `live::switch_live_settings` | `claude_live_to_snapshot`（0..1 条） | `claude_extract_snippet` | 拒凭据键（env 是认证通道） | settings_config 层 + 受控开关域（+ 模板变量拦截） | OpenAI 兼容 |
//! | codex | `config.toml` + `auth.json` | `live_codex::switch_codex_live` | `codex_live_to_snapshot`（0..1 条） | `codex_extract_snippet` | 拒身份键（TOML） | 写盘层 | OpenAI 兼容 |
//! | gemini | `.env` + `settings.json` | `live_gemini::write_gemini_live_at` | `gemini_live_to_snapshot`（0..1 条） | `gemini_extract_snippet` | 拒凭据/端点键 | settings_config 层 + 顶层整体域 | Google 原生 |
//! | grok | `config.toml` | `live_grok::switch_grok_live` | `grok_live_to_snapshot`（0..1 条） | `grok_extract_snippet` | 拒身份键（TOML） | 写盘层 | OpenAI 兼容 |
//! | opencode | `opencode.json`（附加模式：写盘不走 write_live，单键 RMW 在 `live_opencode`） | 防御 Err（走 `live_opencode` 附加模式路径） | `opencode_live_entries`（0..N 条——附加模式多供应商共存） | None（无片段概念） | 恒 Ok | 无片段 | OpenAI 兼容 |
//!
//! settings_config 的形状（`{env, auth, config}` 字段名 / 密钥键名与 typed
//! 值 ⇄ 文本的双向）归 [`crate::provider::settings_codec`]（per-app 形状
//! 单源），不在本 seam 复述。
//!
//! ADR-0010 的片段层策略此前只在注释里被复述约 6 遍、没进代码：现在
//! [`SnippetLayer`] 是唯一表达——claude/gemini 的 **settings_config 层**（片段
//! 先并入供应商配置、随受控写盘落地；变体携带 per-app 合并域）与 codex/grok
//! 的**写盘层**（受控合并之后补缺失进 live 文件——否则被写盘白名单滤掉 → 片段
//! 零效果）由写盘机制决定，不是任意选择；opencode 无片段概念。claude 另由
//! [`App::validates_template_vars`] 表达「settings.json 是字面量 JSON，未物化
//! `${VAR}` 会原样写进 live = 废配置，切换前拦下」（gemini 的 `.env` 由 dotenv
//! 展开 `${VAR}` 是合法引用，不拦）。

use std::path::PathBuf;

use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::{
    import_live, live, live_codex, live_gemini, live_grok, live_opencode, model_fetch, snippet,
};

/// ADR-0010 片段合并层策略（注释 → 代码的唯一表达）。
///
/// - [`SettingsConfig`](SnippetLayer::SettingsConfig)：片段在 settings_config 层
///   并入（写盘前合并进供应商配置，随受控写盘落地）——claude / gemini。变体
///   携带该 app 的合并域（受控区形状）：claude = env + 受控顶层开关白名单、
///   gemini = env + settings.json 顶层整体——层与域是同一条 per-app 事实，收在
///   一个变体里，两处分别声明必然漂移。
/// - [`WriteLayer`](SnippetLayer::WriteLayer)：片段在写盘层补缺失（受控合并之后、
///   写盘之前补进 live 文件，否则被写盘白名单滤掉 → 片段零效果）——codex / grok。
/// - [`NoSnippet`](SnippetLayer::NoSnippet)：无片段概念——opencode（附加模式）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SnippetLayer {
    SettingsConfig(snippet::MergeDomain),
    WriteLayer,
    NoSnippet,
}

impl App {
    /// live 文件路径（顺序固定：claude=[settings.json]，codex=[config.toml,
    /// auth.json]，gemini=[.env, settings.json]，grok=[config.toml]，
    /// opencode=[opencode.json]）。写盘 / 片段提取 / 反向导入共用这一份
    /// 「app → 路径」映射（单一事实来源）。opencode 的单份文件是 opencode.json
    /// ——附加模式的**写盘**不走 write_live（单键 read-modify-write 在
    /// `live_opencode`），但**反向导入**读它，与单激活同一条读取面。
    pub(crate) fn live_paths(self) -> AppResult<Vec<PathBuf>> {
        Ok(match self {
            App::Claude => vec![live::claude_settings_path()?],
            App::Codex => vec![
                live_codex::codex_config_path()?,
                live_codex::codex_auth_path()?,
            ],
            App::Gemini => vec![
                live_gemini::gemini_env_path()?,
                live_gemini::gemini_settings_path()?,
            ],
            App::Grok => vec![live_grok::grok_config_path()?],
            App::OpenCode => vec![live_opencode::opencode_config_path()?],
        })
    }

    /// 写盘入口（路径注入：生产由 `activation::resolve_paths` 解析
    /// [`App::live_paths`] 后调用，测试注入临时目录）。每个 app 保持「只合并
    /// 受控字段、非受控原地保留、写前备份、原子写」同一套语义，具体规格见各
    /// live_* 模块。
    ///
    /// `snippet` 是写盘层片段内容（codex/grok，ADR-0010）：空串即无操作；
    /// settings_config 层应用（claude/gemini）忽略它（片段已在调用方并入
    /// settings_config）。
    pub(crate) fn write_live(
        self,
        paths: &[PathBuf],
        provider: &Provider,
        snippet: &str,
    ) -> AppResult<()> {
        match self {
            App::Claude => live::switch_live_settings(&paths[0], &provider.settings_config),
            App::Codex => live_codex::switch_codex_live(
                &paths[0],
                &paths[1],
                &provider.settings_config,
                snippet,
            ),
            App::Gemini => {
                live_gemini::write_gemini_live_at(&paths[0], &paths[1], &provider.settings_config)
            }
            App::Grok => live_grok::switch_grok_live(&paths[0], &provider.settings_config, snippet),
            // OpenCode 是附加模式，不走 write_live（单激活专属）——增删/切换走
            // set/remove_opencode_provider，由命令层按 is_additive_mode 分派。
            // 这里返回 Err 作防御：误调时明确报错，而非走单激活路径或 panic。
            App::OpenCode => Err(AppError::Config(
                "opencode is additive mode; use set/remove_opencode_provider, not write_live"
                    .into(),
            )),
        }
    }

    /// 读 live 文本 → 0..N 条待导入条目（**multiplicity 进接口**）：单激活应用
    /// 0..1 条（文件缺失 / 空 / 无可识别受控内容 = 0 条）；opencode（附加模式）
    /// 0..N 条——「附加模式 = N 条共存」自此是接口形状的一部分，不再被「至多
    /// 1 条」的旧快照形状挤成 unreachable。preview 与 import 从同一条目列表
    /// 推导（单一事实来源），条目形状见 [`import_live::LiveImportEntry`]。
    pub(crate) fn live_import_entries(
        self,
        texts: &[String],
    ) -> AppResult<Vec<import_live::LiveImportEntry>> {
        match self {
            App::Claude => Ok(import_live::claude_live_to_snapshot(&texts[0])
                .map(import_live::LiveImportEntry::from_snapshot)
                .map(|e| vec![e])
                .unwrap_or_default()),
            App::Codex => Ok(import_live::codex_live_to_snapshot(&texts[0], &texts[1])
                .map(import_live::LiveImportEntry::from_snapshot)
                .map(|e| vec![e])
                .unwrap_or_default()),
            App::Gemini => Ok(import_live::gemini_live_to_snapshot(&texts[0], &texts[1])
                .map(import_live::LiveImportEntry::from_snapshot)
                .map(|e| vec![e])
                .unwrap_or_default()),
            App::Grok => Ok(import_live::grok_live_to_snapshot(&texts[0])
                .map(import_live::LiveImportEntry::from_snapshot)
                .map(|e| vec![e])
                .unwrap_or_default()),
            App::OpenCode => import_live::opencode_live_entries(&texts[0]),
        }
    }

    /// 从 live 提取「可共享键」为片段内容（T6，ADR-0012）；无可提取 → None。
    /// opencode 无片段概念 → None。gemini 只需 env（settings.json 的键不在
    /// 片段名册——ADR-0010「Gemini 片段 = JSON env 对象」，列了只会空欢喜）。
    pub(crate) fn extract_snippet(self, texts: &[String]) -> Option<String> {
        match self {
            App::Claude => import_live::claude_extract_snippet(&texts[0]),
            App::Gemini => import_live::gemini_extract_snippet(&texts[0]),
            App::Codex => import_live::codex_extract_snippet(&texts[0]),
            App::Grok => import_live::grok_extract_snippet(&texts[0]),
            App::OpenCode => None,
        }
    }

    /// 片段校验（set 命令用，ADR-0010 凭据策略按应用分）：
    /// - claude / gemini：合法 JSON 对象（空串=空片段）；拒绝凭据键（env 是
    ///   认证通道）；gemini 另拒端点键 `GOOGLE_GEMINI_BASE_URL`、只认 `env`
    ///   子对象（名册决策：其余顶层键/扁平键不在名册，明确拒绝而非静默通过）、
    ///   要求 env 值为非空字符串。
    /// - codex / grok：合法 TOML；拒绝受控身份键（凭据键不禁——`mcp_servers`
    ///   不经 LLM 端点）。
    /// - opencode：附加模式无片段概念 → `Ok`。
    pub(crate) fn validate_snippet(self, snippet: &str) -> AppResult<()> {
        match self {
            App::Claude => snippet::validate_claude_snippet(snippet),
            App::Gemini => snippet::validate_gemini_snippet(snippet),
            App::Codex => live_codex::validate_codex_snippet(snippet),
            App::Grok => live_grok::validate_grok_snippet(snippet),
            App::OpenCode => Ok(()),
        }
    }

    /// T6：把提取的片段内容合并进现有片段（只补缺失，沿用 ADR-0010 语义——
    /// 已有键不覆盖）：claude / gemini（JSON）用
    /// `snippet::merge_snippet_into_settings`（现有片段为 target、提取为补丁，
    /// 合并域 = 各自的受控区形状）；codex / grok（TOML）用 `fill_missing_table`；
    /// opencode 无片段概念 → 原样返回提取内容。返回合并后的片段文本。
    pub(crate) fn merge_extracted_snippet(
        self,
        existing: &str,
        extracted: &str,
    ) -> AppResult<String> {
        match self {
            App::Claude => snippet::merge_snippet_into_settings(
                existing,
                extracted,
                snippet::MergeDomain::ControlledFields,
            ),
            App::Gemini => snippet::merge_snippet_into_settings(
                existing,
                extracted,
                snippet::MergeDomain::WholeTopLevel,
            ),
            App::Codex | App::Grok => {
                let mut doc = live::parse_toml_or_empty(existing, "existing snippet")?;
                let ext = live::parse_toml_or_empty(extracted, "extracted snippet")?;
                live::fill_missing_table(doc.as_table_mut(), ext.as_table());
                Ok(doc.to_string())
            }
            App::OpenCode => Ok(extracted.to_string()),
        }
    }

    /// ADR-0010 片段合并层策略（唯一表达，见 [`SnippetLayer`]；settings_config
    /// 层变体携带合并域——受控区形状与层同源声明）。
    pub(crate) fn snippet_layer(self) -> SnippetLayer {
        match self {
            App::Claude => SnippetLayer::SettingsConfig(snippet::MergeDomain::ControlledFields),
            App::Gemini => SnippetLayer::SettingsConfig(snippet::MergeDomain::WholeTopLevel),
            App::Codex | App::Grok => SnippetLayer::WriteLayer,
            App::OpenCode => SnippetLayer::NoSnippet,
        }
    }

    /// 切换前是否拦截未物化模板变量：claude 的 settings.json 是字面量 JSON，
    /// `${VAR}` 占位符会原样写进 live = 废配置，切换前拦下；gemini 的 `.env`
    /// 由 dotenv 展开 `${VAR}`（合法引用），codex / grok 的 TOML 不经模板变量
    /// 物化——都不拦。
    pub(crate) fn validates_template_vars(self) -> bool {
        matches!(self, App::Claude)
    }

    /// 模型列表拉取协议：gemini 走 Google 原生 `GET /v1beta/models`
    /// （`model_fetch::fetch_gemini_models`，端点形状固定、`models_url` 不参与）；
    /// 其余 app 走 OpenAI 兼容 `GET /v1/models`（候选 URL 构造 + 遍历，见
    /// `model_fetch::fetch_models`）。错误串标签两条路径同一套（前端按标签
    /// 分桶提示）。
    pub(crate) fn fetch_models(
        self,
        base_url: &str,
        api_key: &str,
        models_url: Option<&str>,
    ) -> AppResult<Vec<String>> {
        match self {
            App::Gemini => model_fetch::fetch_gemini_models(base_url, api_key),
            _ => model_fetch::fetch_models(base_url, api_key, models_url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::testutil;
    use serde_json::Value;

    /// 测试用 Provider（settings_config 由调用方指定）。
    fn provider_for(app: App, settings_config: &str) -> Provider {
        testutil::provider(app, "", "test-provider", settings_config)
    }

    /// 与 [`App::live_paths`] 声明的布局一致的临时路径（测试注入用，写盘 /
    /// 导入 / 往返都走这条路）。
    fn temp_live_paths(app: App, dir: &std::path::Path) -> Vec<PathBuf> {
        match app {
            App::Claude => vec![dir.join("settings.json")],
            App::Codex => vec![dir.join("config.toml"), dir.join("auth.json")],
            App::Gemini => vec![dir.join(".env"), dir.join("settings.json")],
            App::Grok => vec![dir.join("config.toml")],
            App::OpenCode => vec![dir.join("opencode.json")],
        }
    }

    fn parsed(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    /// TOML 语义相等：两边都解析成 `toml::Value` 比较——toml_edit 重写不保证
    /// 字节相同，语义比较才是 round-trip 的判据（生产用 toml_edit 是保注释，
    /// 测试用只读 `toml` 是标准解析 API，两不冲突）。
    fn assert_toml_eq(a: &str, b: &str) {
        let va: toml::Value = toml::from_str(a).unwrap_or_else(|e| panic!("parse {a:?}: {e}"));
        let vb: toml::Value = toml::from_str(b).unwrap_or_else(|e| panic!("parse {b:?}: {e}"));
        assert_eq!(va, vb, "TOML 语义不等:\n--- a ---\n{a}\n--- b ---\n{b}");
    }

    /// 各单激活 app 的 canonical 目标配置（只含受控字段）——从 testutil 的
    /// 样例取（样例本身由 settings_codec build 派生，形状声明只有一处）。
    fn round_trip_targets() -> Vec<(App, String)> {
        [App::Claude, App::Codex, App::Gemini, App::Grok]
            .into_iter()
            .map(|app| (app, testutil::sample_settings_config(app)))
            .collect()
    }

    /// 断言导入条目的 settings_config 与目标语义相等（按 app 形状：JSON 全等 /
    /// auth JSON + config TOML）。
    fn assert_entry_matches_target(app: App, entry: &import_live::LiveImportEntry, target: &str) {
        match app {
            App::Claude | App::Gemini => {
                assert_eq!(parsed(&entry.settings_config), parsed(target), "{app:?}");
            }
            App::Codex => {
                let entry_v = parsed(&entry.settings_config);
                let target_v = parsed(target);
                assert_eq!(entry_v["auth"], target_v["auth"], "codex auth 往返");
                assert_toml_eq(
                    entry_v["config"].as_str().unwrap(),
                    target_v["config"].as_str().unwrap(),
                );
            }
            App::Grok => {
                let entry_v = parsed(&entry.settings_config);
                let target_v = parsed(target);
                assert_toml_eq(
                    entry_v["config"].as_str().unwrap(),
                    target_v["config"].as_str().unwrap(),
                );
            }
            App::OpenCode => unreachable!("单激活往返不含 opencode"),
        }
    }

    // ---------------- write ⇄ import 往返（属性测试，5 个 app 形状）--------

    /// write ⇄ import 往返（单一 seam 上跑生产路径，覆盖 4 个单激活 app
    /// 形状）：把供应商 settings_config 经 seam 的 `write_live` 写进临时 live
    /// 文件(s)，再经 `live_import_entries` 反向导入——受控字段语义不变。
    #[test]
    fn write_snapshot_round_trip_preserves_controlled_fields() {
        for (app, target) in round_trip_targets() {
            let tmp = tempfile::tempdir().unwrap();
            let paths = temp_live_paths(app, tmp.path());
            app.write_live(&paths, &provider_for(app, &target), "")
                .unwrap();
            let texts: Vec<String> = paths
                .iter()
                .map(|p| live::read_live_settings(p).unwrap())
                .collect();
            let entries = app
                .live_import_entries(&texts)
                .unwrap_or_else(|e| panic!("{app:?} 导入失败: {e}"));
            assert_eq!(
                entries.len(),
                1,
                "单激活 app 一份 live → 至多 1 条: {app:?}"
            );
            assert_entry_matches_target(app, &entries[0], &target);
        }
    }

    /// 用户手动的非受控 live 字段经写盘保留、经导入被过滤——往返后条目仍只含
    /// 受控字段（属性测试：非受控永不进入 settings_config）。
    #[test]
    fn round_trip_drops_uncontrolled_live_fields() {
        for (app, target) in round_trip_targets() {
            let tmp = tempfile::tempdir().unwrap();
            let paths = temp_live_paths(app, tmp.path());
            // 种入用户手动配置（非受控字段，形状各 app 不同）。
            match app {
                App::Claude => std::fs::write(
                    &paths[0],
                    r#"{"permissions":{"allow":["Bash"]},"hooks":{"PreToolUse":[{"matcher":"Bash"}]},"model":"claude-sonnet-4-5"}"#,
                )
                .unwrap(),
                App::Codex => std::fs::write(
                    &paths[0],
                    "[mcp_servers.filesystem]\ncommand = \"npx\"\n[web_search]\nenabled = true\n",
                )
                .unwrap(),
                App::Gemini => std::fs::write(&paths[0], "KEEP_ME=1\n").unwrap(),
                App::Grok => std::fs::write(
                    &paths[0],
                    "[models]\ndefault = \"my-custom\"\n\n[model.my-custom]\nmodel = \"grok-3\"\n\n[mcp_servers.fs]\ncommand = \"npx\"\n",
                )
                .unwrap(),
                App::OpenCode => unreachable!("单激活往返不含 opencode"),
            }
            app.write_live(&paths, &provider_for(app, &target), "")
                .unwrap();
            let texts: Vec<String> = paths
                .iter()
                .map(|p| live::read_live_settings(p).unwrap())
                .collect();
            let entries = app
                .live_import_entries(&texts)
                .unwrap_or_else(|e| panic!("{app:?} 导入失败: {e}"));
            assert_eq!(entries.len(), 1, "{app:?} 写盘后应有受控内容可导入");
            assert_entry_matches_target(app, &entries[0], &target);
        }
    }

    /// 登录态版（空受控内容）写盘后无可导入：write → import → 0 条。四个
    /// 单激活 app 同一属性（gemini 会建空 .env + oauth 标记、claude 会写空
    /// 对象——但都没有受控内容，导入判空）。
    #[test]
    fn login_state_write_has_nothing_to_import() {
        for (app, _) in round_trip_targets() {
            let tmp = tempfile::tempdir().unwrap();
            let paths = temp_live_paths(app, tmp.path());
            app.write_live(&paths, &provider_for(app, "{}"), "")
                .unwrap();
            let texts: Vec<String> = paths
                .iter()
                .map(|p| live::read_live_settings(p).unwrap())
                .collect();
            let entries = app.live_import_entries(&texts).unwrap();
            assert!(
                entries.is_empty(),
                "{app:?} 登录态版不应有可导入内容: {:?}",
                entries
            );
        }
    }

    /// opencode 形状的 write ⇄ import 往返（附加模式不进 write_live seam——
    /// 走 live_opencode 单键 RMW + provider_entries，都是命令层生产路径）：
    /// 写入 `provider.<key>` 后反向导入，entry 语义不变、其它 provider 保留。
    #[test]
    fn opencode_write_import_round_trip() {
        let live = r#"{"model":"deepseek/deepseek-chat","provider":{"deepseek":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.deepseek.com","apiKey":"sk-old"}}}}"#;
        let entry = r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.moonshot.cn","apiKey":"sk-new"}}"#;
        let merged = live_opencode::merge_opencode_provider(live, "kimi", entry).unwrap();
        let entries = live_opencode::provider_entries(&merged);
        let (key, value) = entries
            .iter()
            .find(|(k, _)| k == "kimi")
            .expect("kimi 已写入");
        assert_eq!(key, "kimi");
        assert_eq!(
            *value,
            parsed(entry),
            "往返后 entry 语义不变（json5 解析 → serde_json 值相等）"
        );
        assert!(
            entries.iter().any(|(k, _)| k == "deepseek"),
            "附加模式核心不变量：其它 provider 不被取消"
        );
    }

    // ---------------- ADR-0010 片段层策略（注释 → 代码）----------------

    /// 片段层策略落进代码：claude/gemini = settings_config 层（变体携带各自
    /// 受控区形状的合并域）、codex/grok = 写盘层、opencode = 无片段；模板变量
    /// 拦截只属于 claude（settings.json 是字面量 JSON）。
    #[test]
    fn adr_0010_snippet_layer_strategy_is_code() {
        assert_eq!(
            App::Claude.snippet_layer(),
            SnippetLayer::SettingsConfig(snippet::MergeDomain::ControlledFields)
        );
        assert_eq!(
            App::Gemini.snippet_layer(),
            SnippetLayer::SettingsConfig(snippet::MergeDomain::WholeTopLevel)
        );
        assert_eq!(App::Codex.snippet_layer(), SnippetLayer::WriteLayer);
        assert_eq!(App::Grok.snippet_layer(), SnippetLayer::WriteLayer);
        assert_eq!(App::OpenCode.snippet_layer(), SnippetLayer::NoSnippet);
        assert!(App::Claude.validates_template_vars());
        for app in [App::Codex, App::Gemini, App::Grok, App::OpenCode] {
            assert!(!app.validates_template_vars(), "{app:?}");
        }
    }

    /// 写盘层应用（codex/grok）：snippet 经 seam 的 `write_live` 补进 live——
    /// switch_provider_cmd 对它们正是把片段内容透传给 write_live（生产路径）。
    #[test]
    fn write_layer_apps_apply_snippet_through_seam() {
        for app in [App::Codex, App::Grok] {
            let tmp = tempfile::tempdir().unwrap();
            let paths = temp_live_paths(app, tmp.path());
            let target = round_trip_targets()
                .into_iter()
                .find(|(a, _)| *a == app)
                .unwrap()
                .1;
            app.write_live(
                &paths,
                &provider_for(app, &target),
                "[mcp_servers.github]\ncommand = \"npx\"\n",
            )
            .unwrap();
            let live_text = live::read_live_settings(&paths[0]).unwrap();
            assert!(
                live_text.contains("[mcp_servers.github]"),
                "{app:?} 片段的 mcp_servers 应补进 live: {live_text}"
            );
        }
    }

    /// settings_config 层应用（claude/gemini）：`write_live` 忽略写盘 snippet——
    /// 片段只在 settings_config 层并入（命令层对这些 app 恒传空串；这里是
    /// write_live「非写盘层应用忽略 snippet」契约的测试）。
    #[test]
    fn settings_config_layer_apps_ignore_write_snippet() {
        for app in [App::Claude, App::Gemini] {
            let tmp = tempfile::tempdir().unwrap();
            let tmp2 = tempfile::tempdir().unwrap();
            let paths = temp_live_paths(app, tmp.path());
            let paths2 = temp_live_paths(app, tmp2.path());
            let target = round_trip_targets()
                .into_iter()
                .find(|(a, _)| *a == app)
                .unwrap()
                .1;
            app.write_live(&paths, &provider_for(app, &target), "")
                .unwrap();
            app.write_live(
                &paths2,
                &provider_for(app, &target),
                r#"{"env":{"SNIPPET_ONLY":"1"}}"#,
            )
            .unwrap();
            for (p1, p2) in paths.iter().zip(&paths2) {
                assert_eq!(
                    live::read_live_settings(p1).unwrap(),
                    live::read_live_settings(p2).unwrap(),
                    "{app:?} 写盘层 snippet 不得影响输出"
                );
            }
        }
    }

    // ---------------- 片段校验（set 命令生产路径）----------------

    /// claude 片段校验经 seam：拒凭据键、放行非凭据键、空串合法。
    #[test]
    fn validate_claude_snippet_via_seam() {
        assert!(App::Claude
            .validate_snippet(r#"{"includeCoAuthoredBy": false}"#)
            .is_ok());
        assert!(App::Claude.validate_snippet("").is_ok());
        assert!(App::Claude.validate_snippet("   ").is_ok());
        assert!(matches!(
            App::Claude.validate_snippet("{nope"),
            Err(AppError::Config(_))
        ));
        assert!(matches!(
            App::Claude.validate_snippet(r#"[1]"#),
            Err(AppError::Config(_))
        ));
        // env 里的凭据键拒绝（env 是认证通道）；顶层凭据键也拒。
        assert!(App::Claude
            .validate_snippet(r#"{"env": {"ANTHROPIC_AUTH_TOKEN": "sk-x"}}"#)
            .is_err());
        assert!(App::Claude
            .validate_snippet(r#"{"env": {"ANTHROPIC_API_KEY": "sk-x"}}"#)
            .is_err());
        assert!(App::Claude.validate_snippet(r#"{"apiKey": "x"}"#).is_err());
        // 非凭据键放行（模型/端点/开关——供应商赢下无害）。
        assert!(App::Claude
            .validate_snippet(r#"{"env": {"ANTHROPIC_MODEL": "m", "ANTHROPIC_BASE_URL": "u"}}"#)
            .is_ok());
    }

    /// gemini 片段校验经 seam：只认 env 子对象（名册决策，ADR-0010「Gemini
    /// 片段 = JSON env 对象」——其余顶层键/扁平键拒绝而非静默通过）、拒凭据键
    /// 与端点键、env 值须非空字符串。
    #[test]
    fn validate_gemini_snippet_via_seam() {
        assert!(
            App::Gemini
                .validate_snippet(r#"{"GEMINI_MODEL": "m"}"#)
                .is_err(),
            "扁平键不在片段名册，必须拒绝"
        );
        assert!(App::Gemini
            .validate_snippet(r#"{"includeCoAuthoredBy": false}"#)
            .is_err());
        assert!(App::Gemini
            .validate_snippet(r#"{"env": {"GEMINI_MODEL": "m"}}"#)
            .is_ok());
        assert!(App::Gemini
            .validate_snippet(r#"{"env": {"GEMINI_API_KEY": "k"}}"#)
            .is_err());
        assert!(App::Gemini
            .validate_snippet(r#"{"env": {"GOOGLE_GEMINI_BASE_URL": "u"}}"#)
            .is_err());
        assert!(App::Gemini
            .validate_snippet(r#"{"env": {"GEMINI_MODEL": 123}}"#)
            .is_err());
        assert!(App::Gemini
            .validate_snippet(r#"{"env": {"GEMINI_MODEL": "  "}}"#)
            .is_err());
        assert!(App::Gemini
            .validate_snippet(r#"{"env": {"GEMINI_MODEL": "gemini-2.5-flash"}}"#)
            .is_ok());
    }

    /// codex / grok 片段校验经 seam：身份键拒绝（TOML），非受控键放行（含
    /// mcp_servers 凭据——不经 LLM 端点）。
    #[test]
    fn validate_codex_and_grok_snippet_via_seam() {
        assert!(App::Codex.validate_snippet(r#"model = "x""#).is_err());
        assert!(App::Codex
            .validate_snippet(
                r#"[mcp_servers.github]
command = "npx"
env = { GITHUB_PERSONAL_ACCESS_TOKEN = "ghp_x" }"#
            )
            .is_ok());
        assert!(App::Grok
            .validate_snippet(
                r#"[model.cc-one]
model = "x""#
            )
            .is_err());
        assert!(App::Grok
            .validate_snippet(
                r#"[mcp_servers.github]
command = "npx""#
            )
            .is_ok());
    }

    /// T6 提取片段并入现有片段经 seam：只补缺失（已有键不覆盖）——JSON 侧
    /// （claude/gemini）与 TOML 侧（codex/grok）同一语义；opencode 原样返回。
    #[test]
    fn merge_extracted_snippet_via_seam() {
        // claude（JSON）：已有键保留、缺失键补上。
        let merged = App::Claude
            .merge_extracted_snippet(
                r#"{"env":{"ANTHROPIC_MODEL":"mine"}}"#,
                r#"{"env":{"ANTHROPIC_MODEL":"extracted","ANTHROPIC_BASE_URL":"https://x"},"includeCoAuthoredBy":false}"#,
            )
            .unwrap();
        let v = parsed(&merged);
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "mine", "已有键不覆盖");
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://x", "缺失键补上");
        assert_eq!(v["includeCoAuthoredBy"], serde_json::json!(false));
        // gemini 同 JSON 语义。
        let gemini = App::Gemini
            .merge_extracted_snippet(
                r#"{"env":{"GEMINI_MODEL":"mine"}}"#,
                r#"{"env":{"GEMINI_MODEL":"x","GEMINI_EXTRA":"y"}}"#,
            )
            .unwrap();
        let gv = parsed(&gemini);
        assert_eq!(gv["env"]["GEMINI_MODEL"], "mine");
        assert_eq!(gv["env"]["GEMINI_EXTRA"], "y");
        // codex（TOML）：同样只补缺失。
        let merged_toml = App::Codex
            .merge_extracted_snippet(
                "[mcp_servers.a]\ncommand = \"x\"",
                "[mcp_servers.a]\ncommand = \"overwrite\"\n[mcp_servers.b]\ncommand = \"y\"",
            )
            .unwrap();
        assert!(merged_toml.contains("command = \"x\""), "已有 server 保留");
        assert!(merged_toml.contains("[mcp_servers.b]"), "缺失 server 补上");
        // grok 同 TOML 语义。
        let grok = App::Grok
            .merge_extracted_snippet(
                "[tui]\ntheme = \"dark\"",
                "[tui]\nextra = 1\n[mcp_servers.b]\ncommand = \"y\"",
            )
            .unwrap();
        assert!(grok.contains("theme = \"dark\""), "已有键保留: {grok}");
        assert!(grok.contains("extra = 1"), "缺失子键补上: {grok}");
        // opencode 无片段概念：原样返回提取内容。
        assert_eq!(
            App::OpenCode
                .merge_extracted_snippet("whatever", "extracted")
                .unwrap(),
            "extracted"
        );
    }

    // ---------------- 形状与附加模式契约 ----------------

    /// live_paths 形状与写盘 / 导入 / 提取的共用契约一致（单一事实来源）。
    #[test]
    fn live_paths_match_documented_layout() {
        let names = |app: App| {
            app.live_paths()
                .unwrap()
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(App::Claude), vec!["settings.json".to_string()]);
        assert_eq!(
            names(App::Codex),
            vec!["config.toml".to_string(), "auth.json".to_string()]
        );
        assert_eq!(
            names(App::Gemini),
            vec![".env".to_string(), "settings.json".to_string()]
        );
        assert_eq!(names(App::Grok), vec!["config.toml".to_string()]);
        assert_eq!(names(App::OpenCode), vec!["opencode.json".to_string()]);
    }

    /// 导入条目的 multiplicity 契约：单激活 app 空文本 → 0 条、有受控内容 →
    /// 0..1 条；opencode（附加模式）→ 0..N 条——N 条共存是接口形状的一部分。
    #[test]
    fn live_import_entries_multiplicity_contract() {
        // 空文本 → 单激活 0 条；opencode 无 provider 段 → 0 条。
        for app in [App::Claude, App::Codex, App::Gemini, App::Grok] {
            let texts = vec![String::new(); temp_live_paths(app, std::path::Path::new("/x")).len()];
            assert!(
                app.live_import_entries(&texts).unwrap().is_empty(),
                "{app:?} 空文本 → 0 条"
            );
        }
        assert!(
            App::OpenCode
                .live_import_entries(&[r#"{"model":"x"}"#.to_string()])
                .unwrap()
                .is_empty(),
            "opencode 无 provider 段 → 0 条"
        );

        // 附加模式：两个 provider 条目 → 2 条（N 条共存，非单激活的至多 1 条）。
        let two = crate::provider::import_live::tests::opencode_live_json();
        let entries = App::OpenCode
            .live_import_entries(&[two.to_string()])
            .unwrap();
        assert_eq!(entries.len(), 2, "附加模式 N 条共存");
        assert!(
            entries
                .iter()
                .all(|e| e.key == "deepseek" || e.key == "kimi"),
            "去重键 = 配置文件原 key: {:?}",
            entries.iter().map(|e| e.key.clone()).collect::<Vec<_>>()
        );
    }

    /// opencode（附加模式）在 seam 上的契约：单份文件是 opencode.json（反向
    /// 导入读它，与单激活同一条读取面）、片段层 NoSnippet、校验恒 Ok、提取
    /// None、合并原样；write_live 防御性报错（误调时明确失败，不走单激活路径）。
    #[test]
    fn opencode_additive_mode_contract() {
        assert_eq!(
            App::OpenCode.live_paths().unwrap().len(),
            1,
            "opencode.json 是附加模式的单一配置文件"
        );
        assert_eq!(App::OpenCode.snippet_layer(), SnippetLayer::NoSnippet);
        assert!(App::OpenCode.validate_snippet("totally {not json").is_ok());
        assert!(App::OpenCode.extract_snippet(&[]).is_none());
        assert_eq!(
            App::OpenCode
                .merge_extracted_snippet("existing", "extracted")
                .unwrap(),
            "extracted"
        );
        let err = App::OpenCode
            .write_live(&[], &provider_for(App::OpenCode, "{}"), "")
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("additive"), "报错要说明附加模式: {msg}");
    }
}
