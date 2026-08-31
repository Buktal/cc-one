//! TOML 载体的写盘共用机制（codex / grok 两家共用）：live / target 文本解析、
//! 片段键补缺失、片段校验与合并骨架、settingsConfig 的 `config` 字段提取。
//! JSON 载体的对应原语（三态合并、内部键清洗、live/target 解析）归
//! [`crate::provider::live`]——受控轴三态语义是双载体一份契约（见
//! `live::merge_controlled_fields_toml` 的文档），两边各一段机械循环，改动
//! 语义必须同步。本模块的公开项经 [`crate::provider::live`] re-export，既有
//! `live::` 引用面（live_codex / live_grok / live_adapter / settings_codec）
//! 路径零变化。

use toml_edit::{DocumentMut, Table};

use crate::error::{AppError, AppResult};

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

/// 递归表级补缺失：`source` 的键 `target` 没有 → 插入；双方都是 table → 递归
/// 合并子键；其余（target 已有非 table、或类型不一致）→ target 赢，跳过。codex
/// / grok 的写盘层片段补缺失共用（单一事实来源）——保证片段的 `[mcp_servers.a]`
/// 与 live 的 `[mcp_servers.b]` 并存、同子键 live 已有则不覆盖（见 ADR-0010）。
pub(crate) fn fill_missing_table(target: &mut Table, source: &Table) {
    for (key, item) in source.iter() {
        let existing_is_table = target.get(key).is_some_and(|e| e.is_table());
        if existing_is_table && item.is_table() {
            let t = target
                .get_mut(key)
                .and_then(|e| e.as_table_mut())
                .expect("checked table");
            let s = item.as_table().expect("checked table");
            fill_missing_table(t, s);
        } else if target.get(key).is_none() {
            target.insert(key, item.clone());
        }
    }
}

/// TOML 片段校验共用骨架（codex / grok，合并层分派见 ADR-0010）：片段须是
/// 合法 TOML 且不含受控身份键——`identity_hit` 返回命中身份键的描述（用于
/// 报错指明具体键，#55：键名细节只出现在校验报错里；`None` = 未命中）。
/// set 命令的提前拦截与 [`merge_toml_snippet`] 的兜底拒绝走同一条规则。
pub(crate) fn validate_toml_snippet(
    snippet: &str,
    label: &str,
    identity_hit: impl Fn(&DocumentMut) -> Option<String>,
) -> AppResult<()> {
    let doc = parse_toml_or_empty(snippet, &format!("{label} snippet"))?;
    reject_snippet_identity(&doc, label, &identity_hit)
}

/// TOML 片段补缺失共用骨架（codex / grok）：在 merge 结果上
/// [`fill_missing_table`] 补片段键（live 已有则保留、递归进子表），身份键
/// 拒绝与 [`validate_toml_snippet`] 同一条规则（防绕过 set 的路径）。
pub(crate) fn merge_toml_snippet(
    merged: &str,
    snippet: &str,
    label: &str,
    identity_hit: impl Fn(&DocumentMut) -> Option<String>,
) -> AppResult<String> {
    let mut doc = parse_toml_or_empty(merged, "merged config.toml")?;
    let snippet_doc = parse_toml_or_empty(snippet, &format!("{label} snippet"))?;
    reject_snippet_identity(&snippet_doc, label, &identity_hit)?;
    fill_missing_table(doc.as_table_mut(), snippet_doc.as_table());
    Ok(doc.to_string())
}

/// 片段携带受控身份键 → `Err`（报错含 `identity_hit` 给出的具体键）。
fn reject_snippet_identity(
    doc: &DocumentMut,
    label: &str,
    identity_hit: &impl Fn(&DocumentMut) -> Option<String>,
) -> AppResult<()> {
    if let Some(hit) = identity_hit(doc) {
        return Err(AppError::Config(format!(
            "{label} 通用片段不得包含受控身份键 {hit}（身份键归供应商管理）"
        )));
    }
    Ok(())
}

/// 从已剥内部 meta 键的 settingsConfig 对象提取 `config` TOML 字符串（codex /
/// grok 共用，两家的 settingsConfig 形状在此字段上同构；字段名
/// [`CONFIG_TOML_FIELD`] 归 settings_codec 单源）：缺失 → 空串（登录态版 /
/// 无受控内容）；非字符串 → `Err`（坏配置不能进用户 config.toml）。
pub(crate) fn config_toml_field(obj: &serde_json::Value) -> AppResult<String> {
    use crate::provider::settings_codec::CONFIG_TOML_FIELD;
    match obj.get(CONFIG_TOML_FIELD) {
        None => Ok(String::new()),
        Some(v) => v.as_str().map(str::to_string).ok_or_else(|| {
            AppError::Config(format!(
                "provider settingsConfig {CONFIG_TOML_FIELD} must be a TOML string"
            ))
        }),
    }
}
