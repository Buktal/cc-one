//! Provider 域共享测试构造（对应 `db::testutil` 先例）。
//!
//! 每个 `pub(crate) fn` 是一个小构造器，把嘈杂的结构体字面量藏起来，让测试
//! 只读被测行为。`provider` / `provider_with_meta` 构造最小 Provider 行；需要
//! 定制个别字段时用结构体更新语法（`Provider { website_url: ..,
//! ..testutil::provider(..) }`）；`sample_settings_config` 给各 app 的
//! canonical 受控子集样例（写盘 ⇄ 反向解析往返的共用目标形状）；
//! `live_with_uncontrolled` / `toml_get_str` 是 codex / grok 的 TOML live
//! 测试样本与读值器（此前两个测试模块各抄一份，收编为一份）。

use crate::model::{App, Provider, ProviderCategory};

/// 构造一条最小 Provider（展示字段空白、meta 空；id 空串 = `save_provider`
/// 生成 hex id）。个别测试需要定制字段时用结构体更新语法覆盖。
pub(crate) fn provider(app: App, id: &str, name: &str, settings_config: &str) -> Provider {
    provider_with_meta(app, id, name, settings_config, "{}")
}

/// [`provider`] 带 meta 的变体（附加模式的 liveKey / liveManaged、模板变量
/// 记录等测试用它）。
pub(crate) fn provider_with_meta(
    app: App,
    id: &str,
    name: &str,
    settings_config: &str,
    meta: &str,
) -> Provider {
    Provider {
        id: id.into(),
        name: name.into(),
        website_url: String::new(),
        category: ProviderCategory::Custom,
        app,
        icon: String::new(),
        icon_color: String::new(),
        sort_index: 0,
        notes: String::new(),
        settings_config: settings_config.into(),
        meta: meta.into(),
        updated_at: String::new(),
    }
}

/// 各 app 的 canonical settings_config 样例（只含受控字段，第三方供应商形状
/// ——端点 / key / 模型齐全）：写盘 ⇄ 反向解析往返测试的共用目标。**从
/// settings_codec 的 build 半向派生**——形状文本只有 codec 一份声明，样例不
/// 是第二处形状表（codec 改形状，样例自动跟上）。
pub(crate) fn sample_settings_config(app: App) -> String {
    match app {
        App::Claude => crate::provider::settings_codec::build_claude_settings([
            (
                "ANTHROPIC_BASE_URL".to_string(),
                "https://api.moonshot.cn/anthropic".to_string(),
            ),
            ("ANTHROPIC_MODEL".to_string(), "kimi-k2".to_string()),
        ]),
        App::Codex => crate::provider::settings_codec::build_codex_settings(
            Some("sk-codex-x"),
            "model = \"gpt-5.6\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\nbase_url = \"https://api.openai.com/v1\"\nwire_api = \"responses\"\n",
        ),
        App::Gemini => crate::provider::settings_codec::build_gemini_settings(
            [
                (
                    crate::provider::settings_codec::GEMINI_API_KEY_ENV.to_string(),
                    "sk-gem-x".to_string(),
                ),
                (
                    crate::provider::settings_codec::GOOGLE_GEMINI_BASE_URL_ENV.to_string(),
                    "https://generativelanguage.googleapis.com/v1beta".to_string(),
                ),
                ("GEMINI_MODEL".to_string(), "gemini-2.5-flash".to_string()),
            ],
            Some(
                serde_json::json!({ "model": "gemini-2.5-flash" })
                    .as_object()
                    .expect("literal is an object")
                    .clone(),
            ),
        ),
        App::Grok => crate::provider::settings_codec::build_grok_settings(
            "[model.cc-one]\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"sk-grok-x\"\napi_backend = \"responses\"\ncontext_window = 500000\nname = \"xAI\"\n",
        ),
        // 附加模式：settings_config 就是 opencode.json 的单条 provider entry
        // 子树（npm/options），无包装形状。
        App::OpenCode => r#"{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://api.moonshot.cn","apiKey":"sk-oc-x"}}"#.to_string(),
    }
}

/// codex / grok 的 live `config.toml` 测试样本：带用户手动配置（非受控字段，
/// 按 app 各自形状——codex 是 model_providers / mcp_servers / web_search，
/// grok 是 [models] 指针 + 用户自建 profile / mcp_servers），供「非受控字段
/// 原样保留」类测试共用一个取用处。
pub(crate) fn live_with_uncontrolled(app: App) -> String {
    match app {
        App::Codex => r#"# 用户手动的配置：非受控字段
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
        .to_string(),
        App::Grok => r#"# 用户手动的配置
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
        .to_string(),
        _ => unreachable!("TOML live fixture 只有 codex / grok 两个形状"),
    }
}

/// TOML 文本按键路径取值（测试读值用，不依赖 toml_edit 的渲染格式）：
/// 字符串原样，布尔 / 整数转字符串。codex / grok 的测试此前各抄一份（一份只
/// 兜 bool、一份只兜 integer）——收编为一份超集，三类标量都兜。
pub(crate) fn toml_get_str(s: &str, path: &[&str]) -> Option<String> {
    let doc: toml_edit::DocumentMut = s.parse().ok()?;
    let mut cur = doc.get(path[0])?;
    for key in &path[1..] {
        cur = cur.get(*key)?;
    }
    cur.as_str()
        .map(str::to_string)
        .or_else(|| cur.as_bool().map(|b| b.to_string()))
        .or_else(|| cur.as_integer().map(|i| i.to_string()))
}
