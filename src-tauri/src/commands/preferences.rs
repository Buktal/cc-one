//! 偏好域（Settings「通用」卡片 + 托盘 + 关闭行为）。

use tauri::{Manager, State};

use super::AppState;
use crate::config::{CloseBehavior, ConfigData, Language, LightweightExpand, Skin};
use crate::error::AppResult;

/// User-tunable preferences surfaced in the Settings「通用」card.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct Preferences {
    pub close_behavior: CloseBehavior,
    pub collect_interval_secs: u32,
    pub push_interval_secs: u32,
    pub language: Language,
    pub lightweight_expand: LightweightExpand,
    pub lightweight_auto_tuck_secs: u32,
    pub skin: Skin,
}

fn to_preferences(cfg: &ConfigData) -> Preferences {
    Preferences {
        close_behavior: cfg.close_behavior,
        collect_interval_secs: cfg.collect_interval_secs,
        push_interval_secs: cfg.push_interval_secs,
        language: cfg.language,
        lightweight_expand: cfg.lightweight_expand,
        lightweight_auto_tuck_secs: cfg.lightweight_auto_tuck_secs,
        skin: cfg.skin,
    }
}

/// Read the current preferences for the Settings card.
#[tauri::command]
#[specta::specta]
pub fn get_preferences(state: State<'_, AppState>) -> AppResult<Preferences> {
    Ok(to_preferences(&state.config.get()))
}

/// Persist the window-close behavior.
#[tauri::command]
#[specta::specta]
pub fn set_close_behavior(
    state: State<'_, AppState>,
    close_behavior: CloseBehavior,
) -> AppResult<Preferences> {
    let cfg = state.config.update(|c| c.close_behavior = close_behavior)?;
    Ok(to_preferences(&cfg))
}

/// Persist the background-collect interval (seconds, clamped to [5, 3600]).
/// Pure-local cadence — does not touch the network.
#[tauri::command]
#[specta::specta]
pub fn set_collect_interval(state: State<'_, AppState>, seconds: u32) -> AppResult<Preferences> {
    // Bounds are declared once on ConfigData — the scheduler clamps the
    // stored value through the same fn.
    let clamped = ConfigData::clamp_collect_interval_secs(seconds);
    let cfg = state.config.update(|c| c.collect_interval_secs = clamped)?;
    Ok(to_preferences(&cfg))
}

/// Persist the push-to-sync interval (seconds, clamped to [60, 7200]; Synced
/// only). Decoupled from collect so the Git history grows at this
/// rate, not the (shorter) collect rate.
#[tauri::command]
#[specta::specta]
pub fn set_push_interval(state: State<'_, AppState>, seconds: u32) -> AppResult<Preferences> {
    // Same single declaration as the collect interval above.
    let clamped = ConfigData::clamp_push_interval_secs(seconds);
    let cfg = state.config.update(|c| c.push_interval_secs = clamped)?;
    Ok(to_preferences(&cfg))
}

/// Persist the display language and rebuild the tray menu so the
/// "Quit" item follows the new language immediately. The tray item is the only
/// user-facing Rust string; all other UI text is frontend i18n driven by this
/// same preference.
#[tauri::command]
#[specta::specta]
pub fn set_language(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    language: Language,
) -> AppResult<Preferences> {
    let cfg = state.config.update(|c| c.language = language)?;
    if let Some(tray) = app_handle.tray_by_id("main") {
        if let Ok(menu) = crate::tray_menu_for(&app_handle, language) {
            let _ = tray.set_menu(Some(menu));
        }
    }
    Ok(to_preferences(&cfg))
}

/// Swap the tray AND taskbar/Alt-Tab icon between the dark-badge and
/// light-badge PNG so both track the resolved UI theme. Stateless — theme
/// truth lives in next-themes (localStorage); Rust never persists or reads it
/// back. The frontend pushes the *resolved* dark/light value, not the user's
/// "system" choice, because `system` only resolves on the JS side (via the OS
/// theme) and the icon needs a concrete asset now. The taskbar icon is set
/// live via `set_icon`; the Start-menu / exe icon is a packaged resource that
/// can't change at runtime.
#[tauri::command]
#[specta::specta]
pub fn set_tray_theme(app_handle: tauri::AppHandle, dark: bool) -> AppResult<()> {
    if let Some(tray) = app_handle.tray_by_id("main") {
        let bytes: &[u8] = if dark {
            include_bytes!("../../icons/tray-dark.png")
        } else {
            include_bytes!("../../icons/tray-light.png")
        };
        if let Ok(icon) = tauri::image::Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(icon.clone()));
            if let Some(window) = app_handle.get_webview_window("main") {
                // window.set_icon swaps the taskbar + Alt-Tab icon live.
                let _ = window.set_icon(icon);
            }
        }
    }
    Ok(())
}

/// Persist the lightweight half-icon expand trigger. Pure frontend
/// behavior; Rust doesn't read it back, but it rides ConfigData for unity.
#[tauri::command]
#[specta::specta]
pub fn set_lightweight_expand(
    state: State<'_, AppState>,
    lightweight_expand: LightweightExpand,
) -> AppResult<Preferences> {
    let cfg = state
        .config
        .update(|c| c.lightweight_expand = lightweight_expand)?;
    Ok(to_preferences(&cfg))
}

/// Persist the auto-tuck delay (seconds; `0` = off) before an invisible full
/// window morphs into the mini bar. Pure frontend behavior; Rust stores it for
/// unity with the other Settings prefs.
#[tauri::command]
#[specta::specta]
pub fn set_lightweight_auto_tuck(state: State<'_, AppState>, secs: u32) -> AppResult<Preferences> {
    let cfg = state
        .config
        .update(|c| c.lightweight_auto_tuck_secs = secs)?;
    Ok(to_preferences(&cfg))
}

/// Persist the color skin (multi-skin theming). Pure frontend effect — Rust
/// never reads it back; it rides ConfigData for unity with the other prefs.
#[tauri::command]
#[specta::specta]
pub fn set_skin(state: State<'_, AppState>, skin: Skin) -> AppResult<Preferences> {
    let cfg = state.config.update(|c| c.skin = skin)?;
    Ok(to_preferences(&cfg))
}

/// Resolve the one-time close dialog. `remember` pins `choice` as
/// the persisted behavior; the chosen action is then executed immediately.
/// `Minimize`/`Ask` hide the window (scheduler keeps running); `Quit` exits.
#[tauri::command]
#[specta::specta]
pub fn confirm_close(
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    choice: CloseBehavior,
    remember: bool,
) -> AppResult<()> {
    if remember {
        let _ = state.config.update(|c| c.close_behavior = choice);
    }
    match choice {
        CloseBehavior::Quit => app_handle.exit(0),
        CloseBehavior::Minimize | CloseBehavior::Ask => {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.hide();
            }
        }
    }
    Ok(())
}
