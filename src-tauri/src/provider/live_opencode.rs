//! OpenCode 写盘（live_opencode）：JSON 单键 read-modify-write，附加模式。
//!
//! OpenCode 是**附加模式**应用：多个供应商共存于
//! `~/.config/opencode/opencode.json` 的 `provider.<key>` map，无唯一活跃。
//! 这与 claude/codex/gemini/grok 的单激活（`write_live` 受控合并、一个 app
//! 一个活跃）本质不同——故**不进 write_live**（误调在 `write_live` 里直接
//! `Err`），走本模块独立的单键 RMW：
//!
//! - 目标文件：`~/.config/opencode/opencode.json`（**硬编码 `~/.config/opencode`，
//!   不尊重 `XDG_CONFIG_HOME`**——OpenCode CLI 自身在 mac/win 也硬编码此路径，
//!   写到 XDG 位置它不读）。
//! - 结构：顶层 JSON 对象，`provider` 是其下的供应商 map；每个供应商是
//!   `provider.<key> = { npm, options:{baseURL,apiKey,headers}, models }`。
//!   cc one 的投影区是整个 `provider` map 的**单个键** `<key>`；用户的其它
//!   provider 条目 + 顶层字段（`model`/`theme`/`mcp`/`plugin`/`$schema`/任意键）
//!   是非受控，read-modify-write 原样穿过。
//! - 读用 `json5`（容忍用户手写的注释/尾逗号）；写用 pretty + 字母序 key
//!   （`serde_json` 默认 `BTreeMap` 即字母序，对齐 OpenCode CLI 的输出习惯）。
//!   代价：json5 → `serde_json` 重排键序、丢失注释——这是 JSON5 round-trip 的
//!   固有限制，与 grok（`toml_edit` 逐字节保留）不同。「其它字段保留」在这里是
//!   **语义保留**（键值都在，顺序/注释可能变），非字节保留，测试按语义断言。
//! - 写前备份 `opencode.json.bak`（单份覆盖，与 4 个单激活 app 统一；CC-Switch
//!   不备份，cc one 更保守——用户可能手改 opencode.json，备份是安全网）。
//! - 原子写（临时文件 + 改名，进程中断不产生半截文件）。
//! - 无操作判定用**语义比较**（合并前后 `Value` 相等 → 不备份、不写盘）——这样
//!   用户 live 里的注释/键序只要数据没变就不被改写；首次写入把 json5 标准化为
//!   json 是唯一会动注释的情形（无法避免，要插数据就得写）。
//! - 清洗：写盘前剥 settingsConfig 的内部 meta 字段（沿用
//!   `live::LIVE_INTERNAL_KEYS`）。
//!
//! [`merge_opencode_provider`] / [`remove_opencode_provider_key`] 是纯函数
//! （本项目最高价值的测试接缝）：输入 (当前 live JSON 文本, 目标 key, entry
//! JSON 文本) → 输出合并后 JSON 文本，不碰文件系统。「只动这一个键、其它保留」
//! 这个关键不变量靠它们落进可测代码。注意 key（opencode.json 的
//! `provider.<key>`）由命令层决定来源（name slug / id / 专用字段——附加模式
//! 允许改名，key 是否随改名变是命令层策略），本模块只接收最终 key 字符串，不
//! 关心它怎么派生。

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{AppError, AppResult};

/// `~/.config/opencode` 目录（跨平台统一走 home，硬编码不尊重 XDG_CONFIG_HOME）。
/// OpenCode CLI 自身在 mac/win 也硬编码此路径，写到 XDG 位置它不读。
pub fn opencode_config_dir() -> AppResult<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| AppError::Config("cannot resolve home dir".into()))?;
    Ok(home.join(".config").join("opencode"))
}

/// `~/.config/opencode/opencode.json` 路径。
pub fn opencode_config_path() -> AppResult<PathBuf> {
    Ok(opencode_config_dir()?.join("opencode.json"))
}

/// 解析 opencode.json live 文本为 JSON 对象（json5 容忍注释/尾逗号）：空串/
/// 纯空白 → `{}`；非空但非对象（数组/标量）→ `Err`。opencode.json 的顶层还有
/// model/theme/mcp 等用户自有字段，非对象根无法保留它们，宁可报错让用户修，
/// 也不静默重建删掉用户配置（与 CC-Switch `read_opencode_config` 一致）。
fn parse_opencode_live(live: &str) -> AppResult<Value> {
    let trimmed = live.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let v: Value = json5::from_str(trimmed)
        .map_err(|e| AppError::Config(format!("opencode.json is not valid JSON/JSON5: {e}")))?;
    if !v.is_object() {
        return Err(AppError::Config(
            "opencode.json root is not a JSON object".into(),
        ));
    }
    Ok(v)
}

/// 解析供应商 settingsConfig（单 provider 条目 JSON）为「剥过内部 meta 键的
/// 对象」：空串/纯空白 → `Err`（OpenCode 供应商必须有 npm/options 内容，空
/// entry 无意义——附加模式下没有单激活那种「登录态版」）；非对象 → `Err`。剥
/// [`live::LIVE_INTERNAL_KEYS`](crate::provider::live::LIVE_INTERNAL_KEYS)——这些
/// 键只供应用自己读，不是 opencode.json 的合法 provider 字段，绝不落 live。
fn parse_opencode_entry(entry_json: &str) -> AppResult<Value> {
    let trimmed = entry_json.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config(
            "opencode provider settingsConfig must not be empty".into(),
        ));
    }
    let mut obj = crate::provider::live::parse_object(trimmed, "provider settingsConfig")?;
    if let Some(o) = obj.as_object_mut() {
        crate::provider::live::strip_internal_keys(o);
    }
    Ok(obj)
}

/// 取 live 根对象里 `provider` map 的可变引用；不存在或不是对象 → 替换为空对象
/// 后返回。`provider` 段是 cc one 的投影区，非对象（用户写坏成数组/标量）无
/// 意义，归一化为空表再插目标键——与 CC-Switch `set_provider` 一致；用户的
/// model/theme 等其它顶层字段不受影响。调用方保证 `root` 是对象（由
/// [`parse_opencode_live`] 校验）。
fn provider_map_mut(root: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !root.get("provider").is_some_and(Value::is_object) {
        root.as_object_mut()
            .expect("root is an object (guaranteed by parse_opencode_live)")
            .insert("provider".to_string(), Value::Object(Default::default()));
    }
    root.get_mut("provider")
        .and_then(|v| v.as_object_mut())
        .expect("provider just ensured to be an object")
}

/// 单键合并纯函数（最高价值测试接缝）：在 live 的 `provider` map 里
/// insert/replace `<key>` = entry，其它 provider 条目 + 用户顶层字段
/// （model/theme/mcp/plugin/$schema/任意键）**语义保留**（键值都在；json5→json
/// 重排键序、丢注释是固有代价）。不碰文件系统。
///
/// 边界：live 非空但非法 JSON/JSON5 或非对象根 → `Err`（解析不了就没法保留
/// 用户配置）；entry 非法 JSON、非对象、或空串 → `Err`（坏配置不能进用户
/// opencode.json）。
pub fn merge_opencode_provider(live: &str, key: &str, entry_json: &str) -> AppResult<String> {
    let mut root = parse_opencode_live(live)?;
    let entry = parse_opencode_entry(entry_json)?;
    provider_map_mut(&mut root).insert(key.to_string(), entry);
    Ok(serde_json::to_string_pretty(&root)?)
}

/// 单键移除纯函数（最高价值测试接缝）：移除 live 的 `provider.<key>`，其它
/// provider 条目 + 用户顶层字段**语义保留**。目标键不存在 → 无变化（仍输出
/// 重排后的 json）。不碰文件系统。
///
/// 边界：live 非空但非法 JSON/JSON5 或非对象根 → `Err`；provider 段非对象
/// （缺失/数组/标量）→ 视为无 provider，移除无操作（不归一化——移除不需要写，
/// 顶替反而会破坏用户的非对象 provider 段）。
pub fn remove_opencode_provider_key(live: &str, key: &str) -> AppResult<String> {
    let mut root = parse_opencode_live(live)?;
    if root.get("provider").is_some_and(Value::is_object) {
        if let Some(providers) = root.get_mut("provider").and_then(|v| v.as_object_mut()) {
            providers.remove(key);
        }
    }
    Ok(serde_json::to_string_pretty(&root)?)
}

/// 添加/更新写盘全流程（薄壳，按序调用）：解析 entry → 读 live → 合并 → 语义
/// 无变化则无操作 → 备份 → 原子写。「语义无变化」用合并前后 `Value` 相等判定
/// （而非字符串相等）——json5 live 与 json 输出必然字符串不同（注释/键序），
/// 字符串比较会每次都误触发写盘；语义比较保证「数据没变就不改写、不备份」，
/// 用户的注释/键序得以保留。
pub fn set_opencode_provider(
    config_path: &Path,
    key: &str,
    settings_config: &str,
) -> AppResult<()> {
    let live = crate::provider::live::read_live_settings(config_path)?;
    let old = parse_opencode_live(&live)?;
    let merged = merge_opencode_provider(&live, key, settings_config)?;
    let new = parse_opencode_live(&merged)?;
    if old == new {
        return Ok(());
    }
    crate::provider::live::backup_file(config_path)
        .and_then(|()| crate::provider::live::atomic_write_file(config_path, &merged))
}

/// 移除写盘全流程（薄壳）：读 live → 移除 `<key>` → 语义无变化则无操作 →
/// 备份 → 原子写。目标键不在 live 里 → 无操作（不备份、不写盘）。
pub fn remove_opencode_provider(config_path: &Path, key: &str) -> AppResult<()> {
    let live = crate::provider::live::read_live_settings(config_path)?;
    let old = parse_opencode_live(&live)?;
    let merged = remove_opencode_provider_key(&live, key)?;
    let new = parse_opencode_live(&merged)?;
    if old == new {
        return Ok(());
    }
    crate::provider::live::backup_file(config_path)
        .and_then(|()| crate::provider::live::atomic_write_file(config_path, &merged))
}

// ---- 写盘 key 派生（slug）+ 已托管状态（meta）-------------------------------
//
// opencode.json 的 `provider.<key>` 的 key 不等于 cc one 的 Provider.id（id 是
// 固定 8 位 hex，不可读，且导入时会重新生成）。key 由命令层首次按名字 slugify
// 生成、持久化在 meta.liveKey，之后稳定（改名不重算——附加模式 key 稳定才不会
// 弄断用户顶层 `model: "<key>/<model>"` 引用）。导入时直接用配置文件原 key 作
// liveKey，保留用户手写配置。以下纯函数支撑这套派生，不碰文件系统。

/// 把供应商名字转成 opencode.json 的 provider.<key> 候选 key：小写、非 ASCII
/// 字母数字的字符作分隔（连续/首尾不堆连字符）。纯 ASCII 名字（DeepSeek /
/// OpenRouter / GLM 5.1）得到可读 key（deepseek / openrouter / glm-5-1）；含
/// 中文等非 ASCII 的名字得到空串——调用方应回落到 provider id（slug 无法表达
/// 中文，强行转出会是乱码 key）。
pub fn slugify(name: &str) -> String {
    let mut result = String::new();
    let mut prev_dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !result.is_empty() {
            result.push('-');
            prev_dash = true;
        }
    }
    result.trim_end_matches('-').to_string()
}

/// 解析 live opencode.json，返回 `provider` map 现有的全部 key（provider 段非
/// 对象 / 解析失败 → 空 Vec）。纯函数，命令层用它做 slug 冲突检测。
pub fn provider_keys(live: &str) -> Vec<String> {
    let Ok(root) = parse_opencode_live(live) else {
        return Vec::new();
    };
    root.get("provider")
        .and_then(|p| p.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// 解析 live opencode.json，返回 `provider` map 的全部 (key, entry) 对。entry 是
/// 该 provider 的完整 JSON 子树（npm/options/models）。provider 段非对象/解析失败
/// → 空。import from live 用它遍历配置文件里的供应商。键序由 serde_json 的
/// `BTreeMap` 保证（字母序），无需额外排序。
pub fn provider_entries(live: &str) -> Vec<(String, Value)> {
    let Ok(root) = parse_opencode_live(live) else {
        return Vec::new();
    };
    root.get("provider")
        .and_then(|p| p.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// 在已占用 key 列表里把 base 唯一化：base 不在 taken、或 base 就是自己的旧 key
/// `own_key`（同一 provider 改名/重写不算冲突）→ 直接用；否则加 -2/-3/… 后缀直
/// 到不撞。空 base → 空（调用方回落 id）。纯函数。
pub fn dedupe_slug(base: &str, taken: &[String], own_key: Option<&str>) -> String {
    if base.is_empty() {
        return String::new();
    }
    let is_free = |k: &str| own_key == Some(k) || !taken.iter().any(|t| t == k);
    if is_free(base) {
        return base.to_string();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if is_free(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// 派生 opencode.json 的写盘 key（纯函数，可测）：优先沿用 meta 已存的
/// `liveKey`（key 稳定——改名不重算，避免弄断用户顶层
/// `model: "<key>/<model>"` 引用）；首次按 name slugify，纯非 ASCII 名字 slug 为
/// 空 → 回落 provider id；再在 live 现有 key 里唯一化（冲突加 -2/-3）。
pub fn derive_live_key(name: &str, id: &str, meta: &str, live_text: &str) -> String {
    let own_key = meta_live_key(meta);
    let base = match &own_key {
        Some(k) => k.clone(),
        None => {
            let s = slugify(name);
            if s.is_empty() {
                id.to_string()
            } else {
                s
            }
        }
    };
    dedupe_slug(&base, &provider_keys(live_text), own_key.as_deref())
}

/// 读 meta（raw JSON text）里的 `liveManaged`：`Some(true)` = 已写进
/// opencode.json，`Some(false)` = 仅 DB，`None` = 未设置 / 旧数据 / 单激活应用。
/// 空/非法 meta → `None`（宽容：列表读不应因坏 meta 崩溃）。
pub fn meta_live_managed(meta: &str) -> Option<bool> {
    parse_meta(meta).ok()?.get("liveManaged")?.as_bool()
}

/// opencode entry 的显示名（导入/预览共用的单一事实来源）：`entry.name`
/// 非空字符串优先；缺失或空串 → 回退 `provider.<key>`（空名供应商在列表里
/// 不可辨，key 至少唯一可认）。
pub fn entry_display_name(entry: &Value, key: &str) -> String {
    entry
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(key)
        .to_string()
}

/// 读 meta 里的 `liveKey`（opencode.json 的 provider.<key> 的 key）。`None` = 未
/// 设置（首次添加时由命令层 slugify 生成并写回 meta）。
pub fn meta_live_key(meta: &str) -> Option<String> {
    parse_meta(meta)
        .ok()?
        .get("liveKey")?
        .as_str()
        .map(str::to_string)
}

/// 返回设了 `liveKey` + `liveManaged` 的新 meta JSON text，保留 meta 里其它字段
/// （`templateValues` 等）。空 meta 从 `{}` 起步；非对象/非法 meta → `Err`（写
/// 操作严格，暴露问题而非静默丢弃字段）。键按字母序输出（serde_json 默认）。
pub fn with_meta_live_state(meta: &str, key: &str, managed: bool) -> AppResult<String> {
    let mut obj = parse_meta(meta)?;
    let map = obj
        .as_object_mut()
        .expect("parse_meta guarantees an object");
    map.insert("liveKey".to_string(), Value::String(key.to_string()));
    map.insert("liveManaged".to_string(), Value::Bool(managed));
    Ok(serde_json::to_string_pretty(&obj)?)
}

/// 解析 meta（raw JSON text）为对象：空串 → `{}`；非对象/非法 JSON → `Err`。
fn parse_meta(meta: &str) -> AppResult<Value> {
    let trimmed = meta.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    let v: Value = serde_json::from_str(trimmed)
        .map_err(|e| AppError::Config(format!("provider meta is not valid JSON: {e}")))?;
    if !v.is_object() {
        return Err(AppError::Config(
            "provider meta is not a JSON object".into(),
        ));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 一份带用户手动配置（非受控顶层字段）+ 多个已有 provider 的 live
    /// opencode.json。带注释 + 非字母序键，验证「语义保留」（json5→json 会
    /// 重排键序、丢注释，但值都在）。
    fn live_with_uncontrolled() -> String {
        r#"{
  // 用户手动的顶层配置
  "$schema": "https://opencode.ai/config.json",
  "model": "deepseek/deepseek-chat",
  "theme": "dark",
  "mcp": {
    "filesystem": { "command": "npx", "args": ["-y", "server-fs"] }
  },
  "provider": {
    "deepseek": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "https://api.deepseek.com", "apiKey": "old-key" }
    },
    "kimi": {
      "npm": "@ai-sdk/openai-compatible",
      "options": { "baseURL": "https://api.moonshot.cn" }
    }
  }
}
"#
        .to_string()
    }

    /// 一条单 provider entry（settingsConfig 形状）。
    fn entry(base_url: &str, api_key: &str) -> String {
        format!(
            r#"{{"npm":"@ai-sdk/openai-compatible","options":{{"baseURL":"{}","apiKey":"{}"}}}}"#,
            base_url, api_key
        )
    }

    /// 解析合并/写盘输出为 Value（输出是标准合法 JSON，json5 与 serde_json 都
    /// 能解析；用 json5 是为了也能解析带注释的 .bak 快照）。
    fn parsed(s: &str) -> Value {
        json5::from_str(s).unwrap()
    }

    #[test]
    fn merge_inserts_target_key_preserves_others() {
        let live = live_with_uncontrolled();
        let merged =
            merge_opencode_provider(&live, "glm", &entry("https://open.bigmodel.cn", "glm-key"))
                .unwrap();
        let m = parsed(&merged);
        // 新 key 写入。
        assert_eq!(
            m["provider"]["glm"]["options"]["baseURL"],
            "https://open.bigmodel.cn"
        );
        assert_eq!(m["provider"]["glm"]["options"]["apiKey"], "glm-key");
        // 其它 provider 原样保留（语义）。
        assert_eq!(m["provider"]["deepseek"]["options"]["apiKey"], "old-key");
        assert_eq!(
            m["provider"]["kimi"]["options"]["baseURL"],
            "https://api.moonshot.cn"
        );
        // 顶层用户字段原样保留（语义）。
        assert_eq!(m["model"], "deepseek/deepseek-chat");
        assert_eq!(m["theme"], "dark");
        assert_eq!(m["$schema"], "https://opencode.ai/config.json");
        assert_eq!(m["mcp"]["filesystem"]["command"], "npx");
    }

    #[test]
    fn merge_replaces_existing_key_does_not_touch_others() {
        let live = live_with_uncontrolled();
        // deepseek 已存在 → 整体替换该 key 的内容；其它 provider / 顶层字段不动。
        let merged = merge_opencode_provider(
            &live,
            "deepseek",
            &entry("https://new.deepseek.com", "new-key"),
        )
        .unwrap();
        let m = parsed(&merged);
        assert_eq!(
            m["provider"]["deepseek"]["options"]["baseURL"],
            "https://new.deepseek.com"
        );
        assert_eq!(m["provider"]["deepseek"]["options"]["apiKey"], "new-key");
        assert_eq!(
            m["provider"]["kimi"]["options"]["baseURL"],
            "https://api.moonshot.cn"
        );
        assert_eq!(m["model"], "deepseek/deepseek-chat");
    }

    #[test]
    fn merge_into_empty_live_inserts_only_target() {
        let merged =
            merge_opencode_provider("", "glm", &entry("https://open.bigmodel.cn", "k")).unwrap();
        let m = parsed(&merged);
        assert_eq!(
            m["provider"]["glm"]["options"]["baseURL"],
            "https://open.bigmodel.cn"
        );
        // 空 live 不引入任何非受控字段。
        assert!(m.get("model").is_none());
        assert!(m.get("theme").is_none());
    }

    #[test]
    fn merge_normalizes_non_object_provider_section() {
        // provider 段被用户写坏成数组 → 归一化为空对象再插目标键（投影区）。
        let live = r#"{ "provider": [], "model": "keep" }"#;
        let merged = merge_opencode_provider(live, "glm", &entry("https://x", "k")).unwrap();
        let m = parsed(&merged);
        assert!(m["provider"].is_object());
        assert_eq!(m["provider"]["glm"]["options"]["baseURL"], "https://x");
        assert_eq!(m["model"], "keep", "顶层用户字段仍保留");
    }

    #[test]
    fn merge_strips_internal_meta_keys_from_entry() {
        let entry_with_meta =
            r#"{"api_format":"openai","apiFormat":"openai","npm":"@ai-sdk/openai","options":{}}"#;
        let merged = merge_opencode_provider("", "glm", entry_with_meta).unwrap();
        let m = parsed(&merged);
        assert!(
            m["provider"]["glm"].get("api_format").is_none(),
            "api_format 必须被剥"
        );
        assert!(m["provider"]["glm"].get("apiFormat").is_none());
        assert_eq!(m["provider"]["glm"]["npm"], "@ai-sdk/openai");
    }

    #[test]
    fn merge_invalid_live_json_is_error() {
        let r = merge_opencode_provider("{not json", "glm", &entry("https://x", "k"));
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "live 非法 JSON 必须失败——解析不了就没法保留用户配置"
        );
    }

    #[test]
    fn merge_non_object_live_root_is_error() {
        let r = merge_opencode_provider("[1,2,3]", "glm", &entry("https://x", "k"));
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "live 非对象根必须失败"
        );
    }

    #[test]
    fn merge_invalid_entry_json_is_error() {
        let r = merge_opencode_provider("", "glm", "{nope");
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "entry 非法 JSON 必须失败——坏配置不能进用户 opencode.json"
        );
    }

    #[test]
    fn merge_non_object_entry_is_error() {
        let r = merge_opencode_provider("", "glm", "[1,2]");
        assert!(
            matches!(r, Err(AppError::Config(_))),
            "entry 非对象必须失败"
        );
    }

    #[test]
    fn merge_empty_entry_is_error() {
        for raw in ["", "   "] {
            let r = merge_opencode_provider("", "glm", raw);
            assert!(
                matches!(r, Err(AppError::Config(_))),
                "空 entry 必须失败: {raw:?}"
            );
        }
    }

    #[test]
    fn remove_deletes_target_key_preserves_others() {
        let live = live_with_uncontrolled();
        let merged = remove_opencode_provider_key(&live, "deepseek").unwrap();
        let m = parsed(&merged);
        assert!(m["provider"].get("deepseek").is_none(), "目标键已删");
        assert_eq!(
            m["provider"]["kimi"]["options"]["baseURL"], "https://api.moonshot.cn",
            "其它 provider 保留"
        );
        assert_eq!(m["model"], "deepseek/deepseek-chat", "顶层字段保留");
        assert_eq!(m["mcp"]["filesystem"]["command"], "npx");
    }

    #[test]
    fn remove_missing_key_preserves_everything() {
        let live = live_with_uncontrolled();
        let merged = remove_opencode_provider_key(&live, "nonexistent").unwrap();
        let m = parsed(&merged);
        assert_eq!(m["provider"]["deepseek"]["options"]["apiKey"], "old-key");
        assert_eq!(
            m["provider"]["kimi"]["options"]["baseURL"],
            "https://api.moonshot.cn"
        );
        assert_eq!(m["model"], "deepseek/deepseek-chat");
    }

    #[test]
    fn remove_invalid_live_json_is_error() {
        let r = remove_opencode_provider_key("{bad", "glm");
        assert!(matches!(r, Err(AppError::Config(_))));
    }

    #[test]
    fn remove_with_non_object_provider_section_is_noop() {
        // provider 是数组 → remove 无操作，顶层字段保留（不归一化 provider 段）。
        let live = r#"{ "provider": [], "model": "keep" }"#;
        let merged = remove_opencode_provider_key(live, "glm").unwrap();
        let m = parsed(&merged);
        assert_eq!(m["model"], "keep");
    }

    /// 临时目录里放好 opencode.json（模拟用户 live 配置）。
    fn seed(tmp: &Path, config: Option<&str>) -> PathBuf {
        let config_path = tmp.join("opencode.json");
        if let Some(c) = config {
            fs::write(&config_path, c).unwrap();
        }
        config_path
    }

    #[test]
    fn set_writes_provider_and_preserves_user_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some(&live_with_uncontrolled()));

        set_opencode_provider(
            &config_path,
            "glm",
            &entry("https://open.bigmodel.cn", "glm-key"),
        )
        .unwrap();

        let written = parsed(&fs::read_to_string(&config_path).unwrap());
        assert_eq!(
            written["provider"]["glm"]["options"]["baseURL"],
            "https://open.bigmodel.cn"
        );
        assert_eq!(
            written["provider"]["deepseek"]["options"]["apiKey"], "old-key",
            "已有 provider 保留"
        );
        assert_eq!(written["model"], "deepseek/deepseek-chat", "顶层字段保留");
    }

    #[test]
    fn set_is_noop_when_data_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some(&live_with_uncontrolled()));

        // 先 set glm（数据变化 → 写盘，标准化为 json）。
        set_opencode_provider(&config_path, "glm", &entry("https://x", "k")).unwrap();
        let after_first = fs::read_to_string(&config_path).unwrap();

        // 再用「相同 entry」set 同一 key → 数据无变化 → 无操作（不改写、不备份）。
        let bak = tmp.path().join("opencode.json.bak");
        fs::remove_file(&bak).unwrap();
        set_opencode_provider(&config_path, "glm", &entry("https://x", "k")).unwrap();
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            after_first,
            "数据无变化不得改写"
        );
        assert!(!bak.exists(), "数据无变化不得触发备份");
    }

    #[test]
    fn set_backup_created_when_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some(&live_with_uncontrolled()));

        set_opencode_provider(&config_path, "glm", &entry("https://x", "k")).unwrap();

        let bak = tmp.path().join("opencode.json.bak");
        assert!(bak.exists(), "数据变化必须备份");
        // .bak 是写盘前的 live 快照（无 glm）。
        let bak_v = parsed(&fs::read_to_string(&bak).unwrap());
        assert!(bak_v["provider"].get("glm").is_none(), ".bak 是写盘前快照");
    }

    #[test]
    fn set_missing_file_creates_without_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), None);

        set_opencode_provider(&config_path, "glm", &entry("https://x", "k")).unwrap();

        assert!(config_path.exists());
        assert!(
            !tmp.path().join("opencode.json.bak").exists(),
            "live 原本不存在 → 无备份"
        );
        let written = parsed(&fs::read_to_string(&config_path).unwrap());
        assert_eq!(
            written["provider"]["glm"]["options"]["baseURL"],
            "https://x"
        );
    }

    #[test]
    fn remove_writes_and_preserves_others() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some(&live_with_uncontrolled()));

        remove_opencode_provider(&config_path, "deepseek").unwrap();

        let written = parsed(&fs::read_to_string(&config_path).unwrap());
        assert!(written["provider"].get("deepseek").is_none());
        assert_eq!(
            written["provider"]["kimi"]["options"]["baseURL"],
            "https://api.moonshot.cn"
        );
        assert_eq!(written["model"], "deepseek/deepseek-chat");
    }

    #[test]
    fn remove_missing_key_is_noop_no_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = seed(tmp.path(), Some(&live_with_uncontrolled()));

        let before = fs::read_to_string(&config_path).unwrap();
        remove_opencode_provider(&config_path, "nonexistent").unwrap();
        assert_eq!(
            fs::read_to_string(&config_path).unwrap(),
            before,
            "键不存在 → 不改写"
        );
        assert!(
            !tmp.path().join("opencode.json.bak").exists(),
            "无操作 → 无备份"
        );
    }

    #[test]
    fn opencode_paths_point_at_home_config_opencode() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            opencode_config_dir().unwrap(),
            home.join(".config").join("opencode")
        );
        assert_eq!(
            opencode_config_path().unwrap(),
            home.join(".config").join("opencode").join("opencode.json")
        );
    }

    #[test]
    fn ensure_in_live_does_not_cancel_other_providers() {
        // 附加模式核心不变量：add 一个 provider 绝不取消其它已存在的（与单激活
        // 「切换=替换」的根本区别）。
        let live = live_with_uncontrolled();
        let merged = merge_opencode_provider(&live, "glm", &entry("https://x", "k")).unwrap();
        let m = parsed(&merged);
        assert!(
            m["provider"].get("deepseek").is_some(),
            "已有 provider 不被取消"
        );
        assert!(m["provider"].get("kimi").is_some());
        assert!(m["provider"].get("glm").is_some(), "新 provider 已加入");
    }

    #[test]
    fn slugify_ascii_names_readable_non_ascii_empty() {
        assert_eq!(slugify("DeepSeek"), "deepseek");
        assert_eq!(slugify("OpenRouter"), "openrouter");
        assert_eq!(slugify("GLM 5.1"), "glm-5-1");
        // 含中文：ASCII 段保留、中文段丢弃 → kimi（首尾连字符被裁）。
        assert_eq!(slugify("Kimi (月之暗面)"), "kimi");
        // 纯非 ASCII → 空（调用方回落 provider id）。
        assert_eq!(slugify("月之暗面"), "");
        // 首尾/连续分隔符不堆连字符。
        assert_eq!(slugify("--weird-- name"), "weird-name");
        assert_eq!(slugify("   "), "");
    }

    #[test]
    fn dedupe_slug_avoids_collisions_except_own_key() {
        let taken = vec!["deepseek".to_string(), "glm".to_string()];
        // 不冲突 → 原样。
        assert_eq!(dedupe_slug("kimi", &taken, None), "kimi");
        // 冲突 → -2 后缀。
        assert_eq!(dedupe_slug("deepseek", &taken, None), "deepseek-2");
        // 自己的旧 key 不算冲突（同一 provider 改名/重写）。
        assert_eq!(
            dedupe_slug("deepseek", &taken, Some("deepseek")),
            "deepseek"
        );
        // -2 也被占 → 递增到 -3。
        let taken2 = vec!["glm".to_string(), "glm-2".to_string()];
        assert_eq!(dedupe_slug("glm", &taken2, None), "glm-3");
        // 空 base → 空（调用方回落 id）。
        assert_eq!(dedupe_slug("", &taken, None), "");
    }

    #[test]
    fn derive_live_key_first_time_slugifies_name() {
        // 首次：无 own_key → slug(name)；空 live 不冲突。
        assert_eq!(
            derive_live_key("DeepSeek", "a3f2b1c9", "{}", ""),
            "deepseek"
        );
    }

    #[test]
    fn derive_live_key_first_time_conflict_appends_suffix() {
        // 首次但 slug 已被占用 → 加后缀。
        let live = r#"{"provider":{"deepseek":{}}}"#;
        assert_eq!(derive_live_key("DeepSeek", "id1", "{}", live), "deepseek-2");
    }

    #[test]
    fn derive_live_key_non_ascii_name_falls_back_to_id() {
        // 中文 name slug 为空 → 回落 hex id。
        assert_eq!(
            derive_live_key("月之暗面", "a3f2b1c9", "{}", ""),
            "a3f2b1c9"
        );
    }

    #[test]
    fn derive_live_key_reuses_own_key_ignoring_rename() {
        // 已有 own_key → 沿用，即使 name 变了（key 稳定，不随改名重算）。
        let live = r#"{"provider":{"glm":{}}}"#;
        assert_eq!(
            derive_live_key("Zhipu GLM", "id1", r#"{"liveKey":"glm"}"#, live),
            "glm"
        );
    }

    #[test]
    fn provider_keys_lists_existing() {
        let mut keys = provider_keys(&live_with_uncontrolled());
        keys.sort();
        assert_eq!(keys, vec!["deepseek".to_string(), "kimi".to_string()]);
        // provider 段非对象 → 空。
        assert!(provider_keys(r#"{ "provider": [] }"#).is_empty());
        // 无 provider 段 → 空。
        assert!(provider_keys(r#"{ "model": "x" }"#).is_empty());
    }

    #[test]
    fn provider_entries_returns_key_value_pairs() {
        let entries = provider_entries(&live_with_uncontrolled());
        let keys: Vec<&str> = entries.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"deepseek"));
        assert!(keys.contains(&"kimi"));
        // entry 是完整子树（含 options.apiKey）。
        let deepseek = entries
            .iter()
            .find(|(k, _)| k == "deepseek")
            .expect("deepseek entry exists");
        assert_eq!(deepseek.1["options"]["apiKey"], "old-key");
        assert_eq!(deepseek.1["npm"], "@ai-sdk/openai-compatible");
    }

    #[test]
    fn meta_live_state_roundtrip_preserves_other_fields() {
        let m = r#"{"templateValues":{"X":"y"}}"#;
        // 未设置 → None。
        assert_eq!(meta_live_managed(m), None);
        assert_eq!(meta_live_key(m), None);
        // 写 liveKey + liveManaged，保留 templateValues。
        let updated = with_meta_live_state(m, "deepseek", true).unwrap();
        assert_eq!(meta_live_managed(&updated), Some(true));
        assert_eq!(meta_live_key(&updated), Some("deepseek".to_string()));
        let v: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(v["templateValues"]["X"], "y", "其它 meta 字段保留");
        // 空 meta 起步。
        let from_empty = with_meta_live_state("", "kimi", false).unwrap();
        assert_eq!(meta_live_managed(&from_empty), Some(false));
        assert_eq!(meta_live_key(&from_empty), Some("kimi".to_string()));
    }

    #[test]
    fn meta_helpers_tolerate_bad_meta_on_read_strict_on_write() {
        // 读 helper 对非法/非对象 meta 宽容（None）。
        assert_eq!(meta_live_managed("{bad"), None);
        assert_eq!(meta_live_key("[1,2]"), None);
        // 写 helper 对非法 meta 严格（Err）。
        assert!(with_meta_live_state("{bad", "x", true).is_err());
        assert!(with_meta_live_state("[1,2]", "x", true).is_err());
    }
}
