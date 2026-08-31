//! Tauri 事件名单源（Rust 侧）。
//!
//! 事件名是 Rust ↔ TS 之间的隐式契约：emit 只发生在本模块（常量 + 类型化
//! helper），TS 侧的消费名单是同一份名字的镜像，在 `src/app/app-events.ts`
//! ——**改这里的名字必须同改那边**；名字对不上即静默失联（TS 监听不到、
//! Rust 白发，编译器两端都不报错）。
//!
//! 事件名字符串统一 snake_case。通知轨的「谁写完该发哪条」配对契约由
//! `commands::run_blocking` 的 `Emit` 变体声明；本模块只管「名字 + 怎么发」，
//! 不管「谁该发」。

use tauri::Emitter;

/// Store 整体写（采集 / 同步）后的失效信号：前端失效整个 Store 聚合 tag，
/// 所有 Store 派生读（usage / logs / models / devices / sessions /
/// providers）随之 refetch。
pub(crate) const USAGE_CHANGED: &str = "usage_changed";

/// 会话域写（收藏 / 自定义标题 / 分组归属 / 分组 CRUD）后的失效信号。
pub(crate) const SESSIONS_CHANGED: &str = "sessions_changed";

/// 供应商域写（CRUD / 重排 / 切换 / live 加入移除 / live 与 CC-Switch 导入）
/// 后的失效信号。
pub(crate) const PROVIDERS_CHANGED: &str = "providers_changed";

/// 托盘点开主窗口后通知前端退出 lightweight 模式，展示完整 dashboard。
pub(crate) const TRAY_SHOW_MAIN: &str = "tray_show_main";

/// 主窗口关闭被拦（close_behavior = Ask）后通知前端弹「最小化 / 退出」
/// 对话框。
pub(crate) const CLOSE_REQUESTED: &str = "close_requested";

/// Emit [`USAGE_CHANGED`]（所有事件 payload 均为 `()`，前端 handler 无参）。
pub(crate) fn emit_usage_changed(app: &tauri::AppHandle) {
    let _ = app.emit(USAGE_CHANGED, ());
}

/// Emit [`SESSIONS_CHANGED`]。
pub(crate) fn emit_sessions_changed(app: &tauri::AppHandle) {
    let _ = app.emit(SESSIONS_CHANGED, ());
}

/// Emit [`PROVIDERS_CHANGED`]。
pub(crate) fn emit_providers_changed(app: &tauri::AppHandle) {
    let _ = app.emit(PROVIDERS_CHANGED, ());
}

/// Emit [`TRAY_SHOW_MAIN`]。
pub(crate) fn emit_tray_show_main(app: &tauri::AppHandle) {
    let _ = app.emit(TRAY_SHOW_MAIN, ());
}

/// Emit [`CLOSE_REQUESTED`]。
pub(crate) fn emit_close_requested(app: &tauri::AppHandle) {
    let _ = app.emit(CLOSE_REQUESTED, ());
}
