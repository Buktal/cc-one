//! 「从 live 配置文件导入」的落库半：live → 待导入条目 → Provider。
//!
//! 导入管线分两半，本文件是**落库半（导入 seam）**：
//! - **解析半**（子模块 [`parse`]，「读 live 长什么样」）：live 文本 → 快照
//!   （`*_live_to_snapshot`）与可共享片段内容（`*_extract_snippet`），纯函数、
//!   不碰文件系统 / DB；
//! - **落库半**（本文件，「条目怎么进库」）：把解析产物统一成条目
//!   （[`LiveImportEntry`]），preview 与 import 从同一份条目列表推导——预览
//!   承诺与导入交付不再靠两份代码人眼同步；落库走 [`crate::provider::import`]
//!   的 store 层 seam。
//!
//! opencode 的「从配置文件导入」泛化到 claude / codex / gemini / grok
//! （ADR-0012）。单激活应用的 live 是**单份配置**（settings.json / config.toml
//! / .env + settings.json / config.toml），**一份 live → 至多 1 条条目**（0 条
//! = 文件缺失 / 空 / 无可识别身份内容）；opencode 是 `provider.<key>` map（多条
//! 共存）——multiplicity 由 [`App::live_import_entries`] 的接口形状表达（单激活
//! 0..1 条、opencode 0..N 条）。
//!
//! 去重统一走 [`crate::provider::import`] 的 store 层 seam（冲突键策略作参数）：
//! 单激活应用按 **name**（`meta` 无 opencode 的 liveKey，`live_opencode::
//! meta_live_key` 是 opencode 专属）——同 app 同 name → 更新 name /
//! settings_config（保留 id / 展示字段 / meta），否则新建；opencode 按
//! **liveKey**——同 (app, liveKey) → 更新 name / settings_config / meta，
//! 否则新建。name = live 里 base_url 的注册域（去常见 TLD，用户认得出是哪个
//! 供应商，推导规则在解析半），无 base_url → 应用名（"Claude" 等，数据非 i18n）。
//!
//! `import_from_live_texts` / `preview_from_live_texts` 可测，不碰文件系统——
//! texts 由命令壳按 [`App::live_paths`] 顺序读入（缺失文件读为空串 = 0 条）；
//! 文件 IO 在命令层薄壳（`commands::live_import`）。

mod parse;

use std::collections::HashMap;

use serde_json::Value;

use crate::db::Store;
use crate::error::AppResult;
use crate::model::{App, Provider, ProviderCategory};
use crate::provider::import::{self, ImportKeyStrategy};
use crate::provider::live_opencode;

// 解析半的对外出口保持原位：调用方（`live_adapter`、命令层）继续从
// `import_live::*` 取用，use 路径不因拆分而动；实现在子模块 [`parse`]。
pub use parse::{
    claude_extract_snippet, claude_live_to_snapshot, codex_extract_snippet, codex_live_to_snapshot,
    gemini_extract_snippet, gemini_live_to_snapshot, grok_extract_snippet, grok_live_to_snapshot,
    LiveImportSnapshot,
};

// ---------------- 统一导入条目（multiplicity 进 seam）----------------
//
// 「读 live → 0..N 条待导入条目」的统一中间形状：preview 与 import 从同一份
// 条目列表推导（此前单激活走 Option<快照>、opencode 走 provider.<key> map 各
// 解析一遍，预览承诺与导入交付靠两份代码人眼同步）。条目键的语义按 mode 分：
// 单激活 = name（Name 策略去重）、附加模式 = 配置文件原 key（LiveKey 策略）。

/// 一条「待导入条目」（单激活 0..1 条、opencode 0..N 条——条目数量即 mode 的
/// multiplicity，见 [`App::live_import_entries`]）。
#[derive(Debug, Clone)]
pub struct LiveImportEntry {
    /// 去重键：单激活 = name（Name 策略）、opencode = 配置文件原 key（LiveKey
    /// 策略）。预览「新建 vs 更新」与导入冲突规划用同一把键。
    pub key: String,
    /// 显示名（opencode = entry.name 优先、缺失/空串回退 key；单激活 =
    /// base_url 注册域或应用名）。
    pub name: String,
    /// 名字是否由 base_url 的注册域推导（单激活）；opencode 的名字来自
    /// entry.name / key，恒 false。前端理由行只在该标志为 true 时展示
    /// 「名取自 <url>」。
    pub name_derived_from_url: bool,
    /// base_url（预览展示；无 → 空串）。
    pub base_url: String,
    /// settings_config / entry 是否携带凭据（预览「含密钥」徽标；密钥值绝不
    /// 跨边界）。
    pub has_secret: bool,
    /// 可共享键候选（单激活应用导入后可提取为通用片段；opencode 无片段概念
    /// → 空）。
    pub snippet_candidates: Vec<String>,
    /// settingsConfig 文本（opencode = entry 子树 JSON）。
    pub settings_config: String,
    /// meta 文本（opencode = liveKey + liveManaged=true；单激活 = `"{}"`——
    /// Name 策略不更新 meta）。
    pub meta: String,
}

impl LiveImportEntry {
    /// 单激活快照 → 统一条目：key = name（Name 策略的去重键）、meta 空。
    pub(crate) fn from_snapshot(snap: LiveImportSnapshot) -> Self {
        LiveImportEntry {
            key: snap.name.clone(),
            name: snap.name,
            name_derived_from_url: !snap.base_url.is_empty(),
            base_url: snap.base_url,
            has_secret: snap.has_secret,
            snippet_candidates: snap.snippet_candidates,
            settings_config: snap.settings_config,
            meta: "{}".into(),
        }
    }

    /// 条目 → 待落库 Provider（纯函数）：id 空 → `save_provider` 生成；展示
    /// 字段空白，用户可在 UI 补。
    fn into_provider(self, app: App) -> Provider {
        Provider {
            id: String::new(),
            name: self.name,
            website_url: String::new(),
            category: ProviderCategory::Custom,
            app,
            icon: String::new(),
            icon_color: String::new(),
            sort_index: 0,
            notes: String::new(),
            settings_config: self.settings_config,
            meta: self.meta,
            updated_at: String::new(),
        }
    }
}

/// opencode.json 文本 → 0..N 条待导入条目（附加模式：`provider.<key>` map 里
/// 多供应商共存，一条 entry 一条目）。meta 记 liveKey = 原 key + liveManaged =
/// true（反复导入按 liveKey 去重）。
pub fn opencode_live_entries(live_text: &str) -> AppResult<Vec<LiveImportEntry>> {
    let mut entries = Vec::new();
    for (key, entry) in live_opencode::provider_entries(live_text) {
        // 行内显示名与预览同一推导（单一事实来源）：entry.name 非空优先，
        // 缺失或空串 → key。
        entries.push(LiveImportEntry {
            key: key.clone(),
            name: live_opencode::entry_display_name(&entry, &key),
            name_derived_from_url: false,
            base_url: entry
                .pointer("/options/baseURL")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            has_secret: secret_in_entry(&entry),
            snippet_candidates: vec![],
            settings_config: serde_json::to_string(&entry)?,
            meta: live_opencode::with_meta_live_state("", &key, true)?,
        });
    }
    Ok(entries)
}

/// entry 里是否携带凭据（只出布尔，不回取密钥值）：options.apiKey 非空，或
/// options.headers 任一值非空（headers 可携带 Authorization 等认证头）。
fn secret_in_entry(entry: &Value) -> bool {
    if entry
        .pointer("/options/apiKey")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
    {
        return true;
    }
    entry
        .pointer("/options/headers")
        .and_then(|h| h.as_object())
        .is_some_and(|m| {
            m.values()
                .any(|v| v.as_str().is_some_and(|s| !s.is_empty()))
        })
}

/// 「从 live 配置导入」落库（可测，不碰文件系统——texts 由命令壳按
/// [`App::live_paths`] 顺序读入；缺失文件读为空串 = 0 条）：条目 → Provider →
/// 导入 seam。冲突键策略按 mode 分派（附加模式 = LiveKey、单激活 = Name——
/// 分派走 `is_additive_mode`，禁止 app 判等散落）。
/// `name_overrides` = 预览列表行内改名（key → name；单激活 key == name，同一
/// 覆盖语义）。返回写入条数。
pub fn import_from_live_texts(
    store: &Store,
    app: App,
    texts: &[String],
    name_overrides: &HashMap<String, String>,
) -> AppResult<u32> {
    let entries = app.live_import_entries(texts)?;
    if entries.is_empty() {
        return Ok(0);
    }
    let strategy = if app.is_additive_mode() {
        ImportKeyStrategy::LiveKey
    } else {
        ImportKeyStrategy::Name
    };
    let incoming = entries
        .into_iter()
        .map(|mut entry| {
            if let Some(name) = name_overrides.get(&entry.key) {
                entry.name = name.clone();
            }
            entry.into_provider(app)
        })
        .collect::<Vec<_>>();
    let report = import::import_providers(store, &incoming, strategy)?;
    Ok(report.imported)
}

/// 一条预览行：条目 + is_new（去重键是否已在 DB）。
#[derive(Debug, Clone)]
pub struct LiveImportPreviewRow {
    /// 将导入的条目（与 [`import_from_live_texts`] 同源推导）。
    pub entry: LiveImportEntry,
    /// DB 无此去重键 → 新建；有 → 更新。
    pub is_new: bool,
}

/// 「从 live 配置导入」预览核心（可测，不碰文件系统/DB 之外的任何东西——
/// store 只读现有列表判 is_new）：texts → 0..N 条预览行。is_new 的去重键集合
/// 与导入的冲突键策略同一分派（附加模式 = meta.liveKey 集合、单激活 = name
/// 集合）——预览不承诺导入交付不了的判定。
pub fn preview_from_live_texts(
    store: &Store,
    app: App,
    texts: &[String],
) -> AppResult<Vec<LiveImportPreviewRow>> {
    let entries = app.live_import_entries(texts)?;
    let existing_keys: std::collections::HashSet<String> = store
        .list_providers_for(app)?
        .into_iter()
        .filter_map(|p| {
            if app.is_additive_mode() {
                live_opencode::meta_live_key(&p.meta)
            } else {
                Some(p.name)
            }
        })
        .collect();
    Ok(entries
        .into_iter()
        .map(|entry| {
            let is_new = !existing_keys.contains(&entry.key);
            LiveImportPreviewRow { entry, is_new }
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::testutil::mem;

    /// 一份带两个 provider（一个带 name、一个不带）+ 顶层用户字段的 opencode.json。
    /// `pub(crate)`：命令层预览测试与导入测试共用同一夹具（单一事实来源，
    /// 不各留一份漂移）。
    pub(crate) fn opencode_live_json() -> &'static str {
        r#"{
          "model": "deepseek/deepseek-chat",
          "provider": {
            "deepseek": {
              "npm": "@ai-sdk/openai-compatible",
              "name": "DeepSeek",
              "options": { "baseURL": "https://api.deepseek.com", "apiKey": "sk-x" }
            },
            "kimi": {
              "npm": "@ai-sdk/openai-compatible",
              "options": { "baseURL": "https://api.moonshot.cn" }
            }
          }
        }"#
    }

    // ---------------- opencode 反向导入（LiveKey 策略，seam 落库）----------------

    /// 导入把 provider.<key> 反向落库：新建（空 id → 自动 hex）、liveKey=原 key、
    /// liveManaged=true；display name 取 entry.name，无 name 则取 key。
    #[test]
    fn import_opencode_creates_providers_with_live_key_and_managed_flag() {
        let s = mem();
        let n = import_from_live_texts(
            &s,
            App::OpenCode,
            &[opencode_live_json().to_string()],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 2);
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        assert_eq!(providers.len(), 2);
        let by_name: HashMap<String, Provider> = providers
            .iter()
            .map(|p| (p.name.clone(), p.clone()))
            .collect();
        // 带 name 的 → 用 name。
        let ds = by_name.get("DeepSeek").expect("entry.name 作 display name");
        assert_eq!(
            live_opencode::meta_live_key(&ds.meta).as_deref(),
            Some("deepseek"),
            "liveKey = 配置文件原 key"
        );
        assert_eq!(live_opencode::meta_live_managed(&ds.meta), Some(true));
        // 不带 name 的 → 用 key。
        let kimi = by_name.get("kimi").expect("无 name → key 作 display name");
        assert_eq!(
            live_opencode::meta_live_key(&kimi.meta).as_deref(),
            Some("kimi")
        );
        // settingsConfig 是 entry 子树（npm/options）。
        let sc: Value = serde_json::from_str(&ds.settings_config).unwrap();
        assert_eq!(sc["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(sc["options"]["baseURL"], "https://api.deepseek.com");
    }

    /// 反复 import 同一文件 → 按 liveKey 匹配更新，不产生重复。
    #[test]
    fn import_opencode_updates_existing_same_live_key_no_duplicate() {
        let s = mem();
        let texts = [opencode_live_json().to_string()];
        import_from_live_texts(&s, App::OpenCode, &texts, &HashMap::new()).unwrap();
        let n = import_from_live_texts(&s, App::OpenCode, &texts, &HashMap::new()).unwrap();
        assert_eq!(n, 2, "第二次仍处理 2 条（按 liveKey 更新）");
        assert_eq!(
            s.list_providers_for(App::OpenCode).unwrap().len(),
            2,
            "不产生重复"
        );
    }

    /// 行内改名覆盖：nameOverrides（key → name）优先于 entry.name / key。
    #[test]
    fn import_opencode_respects_name_overrides() {
        let s = mem();
        let overrides = HashMap::from([("deepseek".to_string(), "DS 直连".to_string())]);
        let n = import_from_live_texts(
            &s,
            App::OpenCode,
            &[opencode_live_json().to_string()],
            &overrides,
        )
        .unwrap();
        assert_eq!(n, 2);
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        let ds = providers
            .iter()
            .find(|p| live_opencode::meta_live_key(&p.meta).as_deref() == Some("deepseek"))
            .expect("deepseek 存在");
        assert_eq!(ds.name, "DS 直连", "覆盖名优先于 entry.name");
        // 未被覆盖的 key 仍走 entry.name / key 规则。
        let kimi = providers
            .iter()
            .find(|p| live_opencode::meta_live_key(&p.meta).as_deref() == Some("kimi"))
            .expect("kimi 存在");
        assert_eq!(kimi.name, "kimi");
    }

    /// 无 provider 段 → 0 条（顶层用户字段 model 等被忽略，不报错）。
    #[test]
    fn import_opencode_empty_providers_section_is_zero() {
        let s = mem();
        let n = import_from_live_texts(
            &s,
            App::OpenCode,
            &[r#"{"model":"x"}"#.to_string()],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 0);
        assert!(s.list_providers_for(App::OpenCode).unwrap().is_empty());
    }

    // ---------------- 单激活应用导入（Name 策略，seam 落库）----------------

    /// claude live → 条目 → 落库：按 name（base_url 注册域）新建 Provider。
    #[test]
    fn import_from_live_creates_provider_with_registry_domain_name() {
        let s = mem();
        let n = import_from_live_texts(
            &s,
            App::Claude,
            &[r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"kimi"}}"#.to_string()],
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(n, 1);
        let providers = s.list_providers_for(App::Claude).unwrap();
        assert_eq!(providers.len(), 1);
        assert_eq!(
            providers[0].name, "moonshot",
            "注册域去 TLD（host_of 规则）"
        );
        let sc: Value = serde_json::from_str(&providers[0].settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "kimi");
    }

    /// 反复导入同 name → 更新不重复（单激活按 name 去重）。
    #[test]
    fn import_from_live_dedupes_by_name() {
        let s = mem();
        let texts = [r#"{"env":{"ANTHROPIC_MODEL":"m1"}}"#.to_string()];
        import_from_live_texts(&s, App::Claude, &texts, &HashMap::new()).unwrap();
        import_from_live_texts(&s, App::Claude, &texts, &HashMap::new()).unwrap();
        assert_eq!(
            s.list_providers_for(App::Claude).unwrap().len(),
            1,
            "同 name 不产生重复"
        );
    }

    /// Name 策略的更新语义（store 层）：同 name 第二次导入 → 更新
    /// settings_config，保留已有行的 id / 展示字段 / meta。
    #[test]
    fn import_from_live_updates_settings_keeps_id_and_display_fields() {
        let s = mem();
        let first = r#"{"env":{"ANTHROPIC_MODEL":"m1","ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic"}}"#;
        import_from_live_texts(&s, App::Claude, &[first.to_string()], &HashMap::new()).unwrap();
        let row = &s.list_providers_for(App::Claude).unwrap()[0];
        let id = row.id.clone();
        let meta = row.meta.clone();
        assert_eq!(row.name, "moonshot", "注册域作 name");

        let second = r#"{"env":{"ANTHROPIC_MODEL":"m2","ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic"}}"#;
        let n = import_from_live_texts(&s, App::Claude, &[second.to_string()], &HashMap::new())
            .unwrap();
        assert_eq!(n, 1);
        let rows = s.list_providers_for(App::Claude).unwrap();
        assert_eq!(rows.len(), 1, "同 name 不新建");
        assert_eq!(rows[0].id, id, "保留已有 id");
        assert_eq!(rows[0].meta, meta, "meta 保留（Name 策略不更新 meta）");
        assert_eq!(rows[0].name, "moonshot", "name 不变（同 name 更新）");
        let sc: Value = serde_json::from_str(&rows[0].settings_config).unwrap();
        assert_eq!(sc["env"]["ANTHROPIC_MODEL"], "m2", "settings_config 更新");
    }

    // ---------------- 统一预览（与导入同一条目推导）----------------

    /// 预览提取 name / endpoint / 密钥布尔 / 去重键：带 name 用 name，缺 name
    /// 用 key；is_new 按去重键（liveKey）集合判定。条目与导入同源（同一份
    /// live_import_entries 产物）。
    #[test]
    fn preview_lists_entries_with_name_endpoint_and_secret() {
        let s = mem();
        let rows = preview_from_live_texts(&s, App::OpenCode, &[opencode_live_json().to_string()])
            .unwrap();
        assert_eq!(rows.len(), 2, "字母序：deepseek 先于 kimi");
        let ds = &rows[0].entry;
        assert_eq!(ds.key, "deepseek");
        assert_eq!(ds.name, "DeepSeek", "entry.name 作显示名");
        assert_eq!(ds.base_url, "https://api.deepseek.com");
        assert!(ds.has_secret, "options.apiKey 非空 → 含密钥");
        assert!(rows[0].is_new, "DB 无此 liveKey → 新建");
        let kimi = &rows[1].entry;
        assert_eq!(kimi.key, "kimi");
        assert_eq!(kimi.name, "kimi", "无 name → key 作显示名");
        assert_eq!(kimi.base_url, "https://api.moonshot.cn");
        assert!(!kimi.has_secret, "无 apiKey → 不含密钥");
    }

    /// 「新建 vs 更新」判定与导入一致：existing_keys 按 liveKey 集合判定。
    #[test]
    fn preview_classifies_new_vs_update() {
        let s = mem();
        // 种入一条已托管的 deepseek（meta.liveKey = deepseek）。
        s.save_provider(crate::provider::testutil::provider_with_meta(
            App::OpenCode,
            "",
            "DeepSeek",
            r#"{"npm":"@ai-sdk/openai-compatible"}"#,
            r#"{"liveKey":"deepseek","liveManaged":true}"#,
        ))
        .unwrap();
        let rows = preview_from_live_texts(&s, App::OpenCode, &[opencode_live_json().to_string()])
            .unwrap();
        assert!(!rows[0].is_new, "已有同 liveKey → 更新");
        assert!(rows[1].is_new, "无此 liveKey → 新建");
    }

    /// 空 name（`"name": ""`）导入与预览同判：回退 key（与导入共用
    /// entry_display_name，#67）。
    #[test]
    fn import_and_preview_agree_on_empty_name_falling_back_to_key() {
        let live = r#"{
          "provider": {
            "blank": {
              "npm": "@ai-sdk/openai-compatible",
              "name": "",
              "options": { "baseURL": "https://x.dev", "apiKey": "sk-x" }
            }
          }
        }"#;
        let s = mem();
        let rows = preview_from_live_texts(&s, App::OpenCode, &[live.to_string()]).unwrap();
        assert_eq!(rows[0].entry.name, "blank", "预览：空 name → key");
        import_from_live_texts(&s, App::OpenCode, &[live.to_string()], &HashMap::new()).unwrap();
        let providers = s.list_providers_for(App::OpenCode).unwrap();
        assert_eq!(
            providers[0].name, "blank",
            "导入与预览同一显示名（空 name → key，不再存空串）"
        );
    }

    /// headers 也能携带凭据（Authorization 等）→ 计入 has_secret；空值不算。
    #[test]
    fn preview_detects_headers_secret() {
        let s = mem();
        let live = r#"{
          "provider": {
            "h1": { "options": { "headers": { "Authorization": "Bearer abc" } } },
            "h2": { "options": { "headers": { "X-Empty": "" } } },
            "h3": { "options": { "apiKey": "" } }
          }
        }"#;
        let rows = preview_from_live_texts(&s, App::OpenCode, &[live.to_string()]).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].entry.has_secret, "headers 非空值 → 含密钥");
        assert!(!rows[1].entry.has_secret, "headers 空值 → 不含");
        assert!(!rows[2].entry.has_secret, "apiKey 空串 → 不含");
    }

    /// 无 provider 段 / 损坏 JSON5 / 非对象根 → 空（与导入「静默 0 条」一致，
    /// preview 与 import 语义不得分叉）。
    #[test]
    fn preview_empty_or_unparseable_live_is_empty() {
        let s = mem();
        for live in [r#"{"model":"x"}"#, "{bad", "[1,2]", ""] {
            let rows = preview_from_live_texts(&s, App::OpenCode, &[live.to_string()]).unwrap();
            assert!(rows.is_empty(), "输入 {live:?} 应 → 空");
        }
    }

    /// 单激活应用的预览行：0..1 条、key == name（Name 策略去重键）、名字由
    /// base_url 推导时 name_derived_from_url = true、is_new 按 name 集合判定。
    #[test]
    fn preview_single_activate_yields_at_most_one_row() {
        let s = mem();
        let live = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.moonshot.cn/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-x","ANTHROPIC_MODEL":"kimi"}}"#;
        let rows = preview_from_live_texts(&s, App::Claude, &[live.to_string()]).unwrap();
        assert_eq!(rows.len(), 1, "单激活一份 live → 至多 1 条");
        assert_eq!(rows[0].entry.key, "moonshot", "去重键 = name");
        assert!(rows[0].entry.name_derived_from_url, "名字取自 base_url");
        assert!(rows[0].is_new, "DB 无同名 → 新建");
        // 同名落库后再预览 → is_new = false（与导入去重同一把键）。
        import_from_live_texts(&s, App::Claude, &[live.to_string()], &HashMap::new()).unwrap();
        let rows = preview_from_live_texts(&s, App::Claude, &[live.to_string()]).unwrap();
        assert!(!rows[0].is_new, "同 name 已存在 → 更新");
        // 无受控内容 → 0 条（与导入同判）。
        assert!(
            preview_from_live_texts(&s, App::Claude, &["{}".to_string()])
                .unwrap()
                .is_empty()
        );
    }
}
