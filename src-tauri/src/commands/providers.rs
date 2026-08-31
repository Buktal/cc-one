//! Provider 域（供应商）：CRUD、导出/导入、拉模型列表。切换编排已下沉
//! `provider::activation`（架构审查候选③），本模块只是其命令层薄壳。
//! 通用配置片段的 get/set/整理/提取在 [`super::snippet`]，live 反向导入在
//! [`super::live_import`]，CC-Switch 导入在 [`super::ccswitch`]。

use tauri::State;

use super::{run_blocking, AppState, Emit};
use crate::error::AppResult;
use crate::model::{App, Provider};
use crate::provider::activation;
use crate::provider::import::ProviderImportMode;
use crate::provider::import::ProviderImportReport;

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
    crate::events::emit_providers_changed(&app_handle);
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
    run_blocking("delete_provider", Emit::Providers(&app_handle), move || {
        // 附加模式：provider 若已写进 live，先走对称的移除半边（live 撤除 +
        // meta.liveManaged=false + 落库）再删 DB——与停用路径共用
        // activation::remove_from_live 同一入口。meta 半边对即将删除的行是一
        // 次幂等落库：若删除中途失败，行（managed=false）与 live 文件（条目
        // 已撤）状态仍然一致；单激活直接删 DB（其 live 由切换覆盖，无残留
        // 概念）。
        if app.is_additive_mode() {
            if let Some(provider) = store.get_provider(app, &id)? {
                let paths = activation::resolve_paths(app)?;
                activation::remove_from_live(&store, provider, &paths.opencode_config)?;
            }
        }
        store.delete_provider(app, &id)?;
        Ok(())
    })
    .await
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
    crate::events::emit_providers_changed(&app_handle);
    Ok(())
}

/// 切换供应商（核心动作）——薄壳：编排下沉 `provider::activation`。单激活
/// 「切换」与附加模式「加入 live」的组合次序（片段合并层 ADR-0010 分派、受控
/// 写盘、「写盘成功才落激活态」顺序不变量）都在那里表达并可测；本命令只剩
/// blocking 执行 + 失效信号。「保存」只写 DB（save_provider_cmd），本命令才
/// 真正写盘。
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
    run_blocking("switch_provider", Emit::Providers(&app_handle), move || {
        let paths = activation::resolve_paths(app)?;
        activation::activate(&store, &config, app, &id, &paths)
    })
    .await
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
/// 留档用，不经过 git 同步。返回文档里的 provider 数量。读全表 + 写用户选的
/// 目标文件都是重 IO——走 [`run_blocking`] 离开主线程。
#[tauri::command]
#[specta::specta]
pub async fn export_providers_cmd(
    state: State<'_, AppState>,
    include_keys: bool,
    target_path: String,
) -> AppResult<u32> {
    let store = state.store.clone();
    run_blocking("export_providers", Emit::None, move || {
        let providers = store.list_providers()?;
        let doc = crate::provider::export_import::export_document(
            &providers,
            include_keys,
            &crate::time::now_iso(),
        )?;
        std::fs::write(&target_path, doc)?;
        Ok(providers.len() as u32)
    })
    .await
}

/// 从 JSON 文档导入供应商（合并 / 覆盖模式）。`source_path` 是前端 open
/// 对话框选的文件。只写本机 DB（`save_provider`），不触发 providers.json
/// 同步写——导入的 key 只进本机库。返回应用 / 跳过计数。读文件 + 批量落库
/// 走 [`run_blocking`]，成功后发 providers 失效信号。
#[tauri::command]
#[specta::specta]
pub async fn import_providers_cmd(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    source_path: String,
    mode: ProviderImportMode,
) -> AppResult<ProviderImportReport> {
    let store = state.store.clone();
    run_blocking(
        "import_providers",
        Emit::Providers(&app_handle),
        move || {
            let json = std::fs::read_to_string(&source_path)?;
            crate::provider::export_import::apply_import(&store, &json, mode)
        },
    )
    .await
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
    run_blocking("fetch_models", Emit::None, move || {
        app.fetch_models(&base_url, &api_key, models_url.as_deref())
    })
    .await
}
