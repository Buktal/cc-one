//! Provider 域（供应商）：CRUD、切换写盘、导出/导入、拉模型列表。
//! 通用配置片段的 get/set/整理/提取在 [`super::snippet`]，live 反向导入在
//! [`super::live_import`]，CC-Switch 导入在 [`super::ccswitch`]。

use tauri::{Emitter, State};

use super::AppState;
use crate::error::{AppError, AppResult};
use crate::model::{App, Provider};
use crate::provider::export_import::ProviderImportMode;
use crate::provider::export_import::ProviderImportReport;
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
        // liveManaged=true 才需移除（已移除的 provider liveManaged=false，live 里
        // 已没它，跳过免得无谓读写文件）。
        if app.is_additive_mode() {
            if let Some(provider) = store.get_provider(app, &id)? {
                if live_opencode::meta_live_managed(&provider.meta) == Some(true) {
                    if let Some(key) = live_opencode::meta_live_key(&provider.meta) {
                        let path = live_opencode::opencode_config_path()?;
                        live_opencode::remove_opencode_provider(&path, &key)?;
                    }
                }
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
/// 记该应用的激活状态。写盘分派 `write_live(app, provider, snippet)`：claude
/// 走 JSON 受控合并进 `~/.claude/settings.json`，codex 走 TOML 受控合并 +
/// auth.json，gemini 走 env 整块替换 + settings.json 受控合并。各分支语义
/// 一致：只替换受控字段、非受控字段（hooks / MCP / permissions / model /
/// mcp_servers 等）从 live 原地保留，不整文件覆盖、不做 Backfill。
///
/// 通用片段按应用分派合并层（ADR-0010）：claude/gemini 在 settings_config 层
/// 并入（合并前先叠片段；claude 另拦截未物化模板变量——gemini 的 `.env` 由
/// dotenv 展开 `${VAR}` 是合法引用，不拦）；codex/grok 在写盘层补缺失（片段
/// 随 `snippet` 传给 `write_live`，受控合并后补进 live 文件——否则被白名单
/// 滤掉→零效果）。「保存」只写 DB（save_provider_cmd），本命令才真正写盘。
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
        // 单激活：按应用分派片段合并层（ADR-0010）——claude/gemini = settings_config
        // 层（片段先并入供应商配置、再随受控写盘落地）；codex/grok = 写盘层（受控
        // 合并之后、片段补缺失进 live 文件——否则被写盘白名单滤掉→片段零效果）。
        // 片段按 provider 归属的应用读取（claude 池读 claude 片段，存量迁移后即原
        // 全局片段，行为不变）。snippet_for 返回 owned，读 guard 随语句结束释放。
        let snippet_record = config.get().snippet_for(app);
        let write_provider = match app {
            // settings_config 层片段（claude/gemini）：片段先并入供应商配置，再随
            // 受控写盘落地。
            App::Claude | App::Gemini => {
                let settings_config = crate::provider::snippet::apply_snippet(
                    &provider.settings_config,
                    &snippet_record.content,
                    snippet_record.enabled,
                )?;
                // claude 的 settings.json 是字面量 JSON：${VAR} 占位符会原样写进
                // live = 废配置，切换前拦下。gemini 的 .env 由 dotenv 展开 ${VAR}
                // （合法引用，gemini 预设也不用模板变量），不拦。
                if app == App::Claude {
                    crate::provider::live::validate_no_unfilled_template_vars(&settings_config)?;
                }
                Provider {
                    settings_config,
                    ..provider.clone()
                }
            }
            // codex/grok 走写盘层（片段随 write_snippet 传给 write_live，受控合并
            // 后补缺失进 live 文件）。
            _ => provider.clone(),
        };
        // 写盘层片段（codex/grok）：启用 → 片段内容，否则空串（switch_*_live
        // 空串即无操作）。claude/gemini 一律空串（其片段已在 settings_config 层
        // 处理）。
        let write_snippet = match app {
            App::Codex | App::Grok if snippet_record.enabled => snippet_record.content.clone(),
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

/// 获取供应商的可用模型列表。`app` 决定端点格式：claude / codex 走 OpenAI
/// 兼容 `GET /v1/models`，gemini 走 Google 原生 `GET /v1beta/models`。WebView
/// fetch 撞 CORS，所以请求由后端发（ureq）。claude / codex 路径里 `models_url`
/// 非空时精确覆写候选列表（只试这一个）；否则对 baseURL 构造候选 URL（版本段
/// 识别 + 兼容子路径剥离，见 `provider::model_fetch::candidate_models_urls`），
/// 按序尝试首个成功。gemini 路径端点形状固定（`gemini_models_url` 构造单一
/// URL），`models_url` 不参与。错误串带稳定前缀标签（AUTH_FAILED /
/// ENDPOINT_CLOSED / TIMEOUT / BAD_FORMAT / NETWORK），两条路径同一套标签，
/// 前端按标签分桶提示。
#[tauri::command]
#[specta::specta]
pub async fn fetch_models_cmd(
    app: App,
    base_url: String,
    api_key: String,
    models_url: Option<String>,
) -> AppResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        if app == App::Gemini {
            crate::provider::model_fetch::fetch_gemini_models(&base_url, &api_key)
        } else {
            crate::provider::model_fetch::fetch_models(&base_url, &api_key, models_url.as_deref())
        }
    })
    .await
    .map_err(|e| AppError::Internal(format!("fetch_models task failed: {e}")))?
}
