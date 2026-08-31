//! 托盘：菜单构建（标签随显示语言）、左键 / 菜单 Show 唤起主窗口、Quit
//! 退出。boot 清单的 `build_tray` 步骤负责创建；`set_language` 在语言切换
//! 时重建菜单——crate 根 re-export 的 [`tray_menu_for`] 是那条既有调用
//! 路径，签名不动。

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

use super::{tauri_err, AppResult, BootCtx};
use crate::config;

/// Tray "Show" label for the active display language. The tray menu
/// items are the ONLY user-facing Rust strings — all other UI text is frontend
/// i18n — so this and [`quit_label`] are rebuilt on a live language change.
fn show_label(lang: config::Language) -> &'static str {
    match lang {
        config::Language::En => "Show",
        config::Language::Zh => "显示",
        config::Language::Ja => "表示",
    }
}

/// Tray "Quit" label for the active display language.
fn quit_label(lang: config::Language) -> &'static str {
    match lang {
        config::Language::En => "Quit",
        config::Language::Zh => "退出",
        config::Language::Ja => "終了",
    }
}

/// (Re)build the tray menu, localized to `lang`. Called at setup
/// and from `set_language` on a language change so both items track the
/// language without an app restart.
///
/// Two items: "Show" surfaces the dashboard; "Quit" exits.
/// The Show entry is the ONLY reliable restore path on Linux, where
/// libappindicator does not emit tray click events — so the left-click handler
/// (`on_tray_icon_event` below) is dead code there and the menu Show item is
/// the fallback. On Windows/macOS left-click stays the primary path and Show is
/// a convenience.
pub(crate) fn tray_menu_for(
    app: &tauri::AppHandle,
    lang: config::Language,
) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", show_label(lang), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", quit_label(lang), true, None::<&str>)?;
    Menu::with_items(app, &[&show, &quit])
}

/// Surface the main window from the tray: show + unminimize + focus, then tell
/// the frontend to morph out of lightweight mode so the FULL
/// dashboard is shown. Shared by the tray menu "Show" entry and the tray
/// left-click handler. `unminimize` covers the GNOME/Mutter case where a hidden
/// window needs both show and unminimize to reliably take focus.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        crate::events::emit_tray_show_main(app);
    }
}

/// `build_tray` 步骤：建托盘。Fatal——拆分前就是 `?`（托盘起不来整个应用
/// 不启动）；该不该降级为 BestEffort 是产品决策，本次不动。
pub(super) fn build(ctx: &mut BootCtx) -> AppResult<()> {
    let menu = tray_menu_for(&ctx.app, ctx.config().get().language).map_err(tauri_err)?;
    // Tray starts on the dark badge (next-themes `defaultTheme="dark"`);
    // the frontend pushes `set_tray_theme` once it resolves the actual
    // theme on mount, so the icon follows light/system/dark afterwards.
    let _tray = TrayIconBuilder::with_id("main")
        .tooltip("CC One")
        .icon(
            tauri::image::Image::from_bytes(include_bytes!("../../icons/tray-dark.png"))
                .expect("embedded tray icon"),
        )
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // No-op on Linux: libappindicator does not emit tray click
            // events, so the menu "Show" entry is the restore path there
            // (see `tray_menu_for`). On Windows/macOS this left-click is
            // the primary path.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(&ctx.app)
        .map_err(tauri_err)?;
    Ok(())
}
