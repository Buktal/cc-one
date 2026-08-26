//! 通用配置片段域：按应用的片段读写、TOML 整理、导入后从 live 提取。

use tauri::State;

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::model::{App, CommonConfigSnippet};
use crate::provider::live;

/// 读某应用的通用配置片段（内容 + 启用开关）。按应用各存一份（claude /
/// codex / gemini），存本机 config.json。缺省键按应用回退默认（claude 为
/// 隐藏署名片段，其余为空片段）。
#[tauri::command]
#[specta::specta]
pub fn get_common_config_snippet_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<CommonConfigSnippet> {
    Ok(state.config.get().snippet_for(app))
}

/// 保存某应用的通用配置片段。内容必须是合法 JSON 对象（空串视为空片段）；
/// 非法 JSON 拒绝保存（`AppError::Config`）。写盘合并只认受控字段，非受控
/// 键在写盘时被忽略。
#[tauri::command]
#[specta::specta]
pub fn set_common_config_snippet_cmd(
    state: State<'_, AppState>,
    app: App,
    json: String,
    enabled: bool,
) -> AppResult<CommonConfigSnippet> {
    app.validate_snippet(&json)?;
    let snippet = CommonConfigSnippet {
        enabled,
        content: json,
    };
    state.config.update(|c| c.set_snippet(app, snippet))?;
    Ok(state.config.get().snippet_for(app))
}

/// TOML 片段整理（「整理」按钮）：taplo 保留注释地格式化（codex/grok 片段）。
/// 纯展示操作，不落盘、不校验身份键——只把文本排版成可读多行。
#[tauri::command]
#[specta::specta]
pub async fn format_toml_cmd(text: String) -> AppResult<String> {
    tauri::async_runtime::spawn_blocking(move || Ok(crate::provider::snippet::format_toml(&text)))
        .await
        .map_err(|e| AppError::Internal(format!("format_toml task failed: {e}")))?
}

// ---------------- 导入后提取通用配置片段（T6）----------------

/// 按 app 读 live 文件(s)，提取「可共享键」为片段内容（分派在
/// `live_adapter` 的 [`App::extract_snippet`]；无可提取 → None）。opencode 无
/// 片段概念 → None。gemini 只需 env（settings.json 的非受控键进片段零效果）。
fn read_live_snippet_extract(app: App) -> AppResult<Option<String>> {
    let Some(texts) = live::read_app_live_texts(app)? else {
        return Ok(None);
    };
    Ok(app.extract_snippet(&texts))
}

/// 导入后「提取为通用片段」（T6，ADR-0012）：读该应用 live 配置的可共享键，
/// 合并进现有片段（已有键不覆盖，沿用 ADR-0010 只补缺失，分派在
/// `live_adapter` 的 [`App::merge_extracted_snippet`]）。启停状态不变——提取是
/// 内容操作，不改用户显式的启停选择（原实现强制 enabled=true 会覆盖显式
/// 停用）。合并结果经与手动保存同一条校验（[`App::validate_snippet`] 单一入口，
/// 提取器滤凭据/端点/空值后这里是兜底）。无可提取 → 现有片段原样。非静默——
/// 前端先检测候选、用户确认才调本命令。返回更新后的片段。
#[tauri::command]
#[specta::specta]
pub fn extract_snippet_from_live_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<CommonConfigSnippet> {
    let current = state.config.get().snippet_for(app);
    let Some(extracted) = read_live_snippet_extract(app)? else {
        return Ok(current);
    };
    let content = app.merge_extracted_snippet(&current.content, &extracted)?;
    app.validate_snippet(&content)?;
    let snippet = CommonConfigSnippet {
        enabled: current.enabled,
        content,
    };
    state.config.update(|c| c.set_snippet(app, snippet))?;
    Ok(state.config.get().snippet_for(app))
}
