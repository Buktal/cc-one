//! Provider 域（供应商）：CRUD、切换写盘、导出/导入、拉模型列表。
//! 通用配置片段的 get/set/整理/提取在 [`super::snippet`]，live 反向导入在
//! [`super::live_import`]，CC-Switch 导入在 [`super::ccswitch`]。

use tauri::{Emitter, State};

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::import::ProviderImportMode;
use crate::provider::import::ProviderImportReport;
use crate::provider::live_adapter::SnippetLayer;
use crate::provider::live_opencode;

/// Emit `providers_changed` so the frontend's provider queries invalidate.
/// `pub(super)`: live-import / CC-Switch 写库命令也走同一失效信号。
pub(super) fn emit_providers_changed(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit("providers_changed", ());
}

/// 列出一个应用池的供应商（app 必填——前端传当前分段 tab，后端按池过滤）。
#[tauri::command]
#[specta::specta]
pub fn list_providers_cmd(state: State<'_, AppState>, app: App) -> AppResult<Vec<Provider>> {
    state.store.list_providers_for(app)
}

/// Upsert a provider (empty id = create, non-empty = edit). Returns the
/// persisted row so the caller learns the assigned id / sort position without
/// a second read. The provider carries its `app` (the pool it belongs to).
#[tauri::command]
#[specta::specta]
pub fn save_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    provider: Provider,
) -> AppResult<Provider> {
    let saved = state.store.save_provider(provider)?;
    emit_providers_changed(&app_handle);
    Ok(saved)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<()> {
    let store = state.store.clone();
    tauri::async_runtime::spawn_blocking(move || -> AppResult<()> {
        // 附加模式：provider 若已写进 live，先从 live 移除再删 DB，避免配置文件
        // 残留 orphan 条目；单激活直接删 DB（其 live 由切换覆盖，无残留概念）。
        // 撤除序列（managed 判定 + key 读取 + 移除写盘）收口在
        // live_opencode::remove_from_live_if_managed，与停用路径（live_import）
        // 共用同一份。
        if app.is_additive_mode() {
            if let Some(provider) = store.get_provider(app, &id)? {
                live_opencode::remove_from_live_if_managed(&provider)?;
            }
        }
        store.delete_provider(app, &id)?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Internal(format!("delete_provider task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn reorder_providers_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    ordered_ids: Vec<String>,
) -> AppResult<()> {
    state.store.reorder_providers(app, &ordered_ids)?;
    emit_providers_changed(&app_handle);
    Ok(())
}

/// 切换供应商（核心动作）：按 (app, id) 查 provider → 按应用分派写盘 →
/// 记该应用的激活状态。写盘分派 `write_live(app, provider, snippet)` 与片段
/// 合并层（ADR-0010）都收口在 `provider::live_adapter` 的 per-app 方法（单一
/// seam，见 [`App::snippet_layer`] / [`App::validates_template_vars`]）：
/// claude/gemini 片段在 settings_config 层并入（先叠片段再写盘），codex/grok
/// 在写盘层补缺失（片段随 `snippet` 透传 `write_live`，受控合并后补进 live
/// 文件——否则被白名单滤掉→零效果）。各分支语义一致：只替换受控字段、非受控
/// 字段（hooks / MCP / permissions / model / mcp_servers 等）从 live 原地保留，
/// 不整文件覆盖、不做 Backfill。「保存」只写 DB（save_provider_cmd），本命令
/// 才真正写盘。
#[tauri::command]
#[specta::specta]
pub async fn switch_provider_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    app: App,
    id: String,
) -> AppResult<Provider> {
    let store = state.store.clone();
    let config = state.config.clone();
    let provider = tauri::async_runtime::spawn_blocking(move || -> AppResult<Provider> {
        let provider = store
            .get_provider(app, &id)?
            .ok_or_else(|| AppError::Config(format!("provider not found in {app:?} pool: {id}")))?;
        // 附加模式（OpenCode）：ensure-in-live——写进 opencode.json + 设
        // liveManaged=true，**不取消其它 provider、不碰 active_providers**（附加
        // 模式无唯一激活）。返回更新了 meta 的 provider。
        if app.is_additive_mode() {
            return super::live_import::ensure_opencode_in_live(&store, provider);
        }
        // 单激活：片段合并层按应用的 ADR-0010 策略分派（策略本身收口在
        // live_adapter::App::snippet_layer，单一 seam——claude/gemini =
        // settings_config 层、codex/grok = 写盘层）。片段按 provider 归属的应用
        // 读取（claude 池读 claude 片段，存量迁移后即原全局片段，行为不变）。
        // snippet_for 返回 owned，读 guard 随语句结束释放。
        let snippet_record = config.get().snippet_for(app);
        let write_provider = match app.snippet_layer() {
            // settings_config 层（claude/gemini）：片段先并入供应商配置，再随
            // 受控写盘落地。claude 的 settings.json 是字面量 JSON：${VAR} 占位符
            // 会原样写进 live = 废配置，切换前拦下（gemini 的 .env 由 dotenv
            // 展开 ${VAR} 是合法引用，不拦——见 App::validates_template_vars）。
            SnippetLayer::SettingsConfig => {
                let settings_config = crate::provider::snippet::apply_snippet(
                    &provider.settings_config,
                    &snippet_record.content,
                    snippet_record.enabled,
                )?;
                if app.validates_template_vars() {
                    crate::provider::live::validate_no_unfilled_template_vars(&settings_config)?;
                }
                Provider {
                    settings_config,
                    ..provider.clone()
                }
            }
            // 写盘层（codex/grok）与无片段（opencode，先于此处返回）：供应商配置
            // 原样进写盘，片段随 write_snippet 走写盘层补缺失。
            SnippetLayer::WriteLayer | SnippetLayer::NoSnippet => provider.clone(),
        };
        // 写盘层片段（codex/grok）：启用 → 片段内容，否则空串（switch_*_live
        // 空串即无操作）。settings_config 层应用一律空串（其片段已在上面并入
        // 供应商配置）。
        let write_snippet = match app.snippet_layer() {
            SnippetLayer::WriteLayer if snippet_record.enabled => snippet_record.content.clone(),
            _ => String::new(),
        };
        crate::provider::live::write_live(app, &write_provider, &write_snippet)?;
        config.update(|c| c.set_active_provider(app, &id))?;
        Ok(provider)
    })
    .await
    .map_err(|e| AppError::Internal(format!("switch_provider task failed: {e}")))??;
    emit_providers_changed(&app_handle);
    Ok(provider)
}

/// 当前激活的完整 provider（前端「当前使用」光卡用，按应用查询）。未激活、
/// 或激活的 provider 已被删除 → `None`。
#[tauri::command]
#[specta::specta]
pub fn get_active_provider_cmd(
    state: State<'_, AppState>,
    app: App,
) -> AppResult<Option<Provider>> {
    let id = match state.config.get().active_provider_id_for(app) {
        Some(id) => id,
        None => return Ok(None),
    };
    state.store.get_provider(app, &id)
}

/// 导出全部供应商为 JSON 文档，写入 `target_path`（前端 save 对话框选的位置）。
/// `include_keys=false` 时剔除 settingsConfig env 里的密钥键。换设备迁移 /
/// 留档用，不经过 git 同步。返回文档里的 provider 数量。
#[tauri::command]
#[specta::specta]
pub fn export_providers_cmd(
    state: State<'_, AppState>,
    include_keys: bool,
    target_path: String,
) -> AppResult<u32> {
    let providers = state.store.list_providers()?;
    let doc = crate::provider::export_import::export_document(
        &providers,
        include_keys,
        &crate::time::now_iso(),
    )?;
    std::fs::write(&target_path, doc)?;
    Ok(providers.len() as u32)
}

/// 从 JSON 文档导入供应商（合并 / 覆盖模式）。`source_path` 是前端 open
/// 对话框选的文件。只写本机 DB（`save_provider`），不触发 providers.json
/// 同步写——导入的 key 只进本机库。返回应用 / 跳过计数。
#[tauri::command]
#[specta::specta]
pub fn import_providers_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    source_path: String,
    mode: ProviderImportMode,
) -> AppResult<ProviderImportReport> {
    let json = std::fs::read_to_string(&source_path)?;
    let report = crate::provider::export_import::apply_import(&state.store, &json, mode)?;
    emit_providers_changed(&app_handle);
    Ok(report)
}

/// 获取供应商的可用模型列表。端点协议按应用分派收口在 `live_adapter` 的
/// [`App::fetch_models`]（单一 seam）：gemini 走 Google 原生
/// `GET /v1beta/models`（端点形状固定，`models_url` 不参与）；其余 app 走
/// OpenAI 兼容 `GET /v1/models`（`models_url` 非空时精确覆写候选列表；否则
/// 对 baseURL 构造候选 URL，按序尝试首个成功，见 `provider::model_fetch`）。
/// WebView fetch 撞 CORS，所以请求由后端发（ureq）。错误串带稳定前缀标签
/// （AUTH_FAILED / ENDPOINT_CLOSED / TIMEOUT / BAD_FORMAT / NETWORK），两条
/// 路径同一套标签，前端按标签分桶提示。
#[tauri::command]
#[specta::specta]
pub async fn fetch_models_cmd(
    app: App,
    base_url: String,
    api_key: String,
    models_url: Option<String>,
) -> AppResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        app.fetch_models(&base_url, &api_key, models_url.as_deref())
    })
    .await
    .map_err(|e| AppError::Internal(format!("fetch_models task failed: {e}")))?
}
