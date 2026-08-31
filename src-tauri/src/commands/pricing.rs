//! 定价域：条目 CRUD、pricing.json 导入导出、LiteLLM 上游拉取、补计费。

use tauri::State;

use super::{run_blocking, AppState, Emit};
use crate::error::{AppError, AppResult};
use crate::model::PricingEntry;

/// Rebill zero-cost rows whose model now has a price (top-up).
#[tauri::command]
#[specta::specta]
pub fn rebill_zero_cost(state: State<'_, AppState>) -> AppResult<u32> {
    let book = state.store.load_pricing_book()?;
    Ok(state.store.rebill_zero_cost(&book)? as u32)
}

#[tauri::command]
#[specta::specta]
pub fn list_pricing(state: State<'_, AppState>) -> AppResult<Vec<PricingEntry>> {
    state.store.list_pricing()
}

/// Add or update a pricing entry from the UI (user edits ⇒ `is_builtin=false`).
#[tauri::command]
#[specta::specta]
pub fn save_pricing_entry(
    state: State<'_, AppState>,
    entry: PricingEntry,
    is_builtin: Option<bool>,
) -> AppResult<()> {
    let mut entry = entry;
    entry.is_builtin = is_builtin.unwrap_or(false);
    state.store.upsert_pricing(&entry)
}

#[tauri::command]
#[specta::specta]
pub fn delete_pricing_entry(state: State<'_, AppState>, model_key: String) -> AppResult<()> {
    state.store.delete_pricing(&model_key)
}

/// Re-load pricing from the local `pricing.json` into the DB. Pricing is
/// per-device local (never synced); this is an import surface, not a sync path.
#[tauri::command]
#[specta::specta]
pub fn reload_pricing_from_file(state: State<'_, AppState>) -> AppResult<u32> {
    let path = state.config.paths().pricing_json();
    if !path.exists() {
        return Err(AppError::Pricing(format!(
            "pricing.json not found at {}",
            path.display()
        )));
    }
    state.store.reload_pricing_from_path(&path)
}

/// Persist current DB pricing to the local `pricing.json` (never synced).
#[tauri::command]
#[specta::specta]
pub fn save_pricing_to_file(state: State<'_, AppState>) -> AppResult<()> {
    let entries = state.store.load_pricing_models()?;
    let doc = crate::pricing::write_pricing_doc(&entries)?;
    let path = state.config.paths().pricing_json();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, doc)?;
    Ok(())
}

/// Fetch LiteLLM upstream pricing and merge into the DB (seed).
/// Network → offloaded via [`run_blocking`]. Best-effort: returns count merged
/// (0 offline).
#[tauri::command]
#[specta::specta]
pub async fn fetch_litellm_pricing(state: State<'_, AppState>) -> AppResult<u32> {
    let store = state.store.clone();
    run_blocking("fetch_litellm_pricing", Emit::None, move || {
        let entries = crate::pricing::fetch_litellm()?;
        let mut merged = 0u32;
        for e in &entries {
            store.upsert_pricing(&e.to_entry())?;
            merged += 1;
        }
        Ok(merged)
    })
    .await
}
