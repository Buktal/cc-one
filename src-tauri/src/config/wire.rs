//! 跨 Rust→JS 边界的偏好取值枚举（`ConfigData` 各偏好字段的类型）。
//!
//! 这些枚举只描述「用户选了什么」：经 serde / specta 的 typed 契约进出
//! config.json 与生成的 bindings.ts，Rust 侧仅存储转发，行为全在前端。
//! 收拢在 `wire` 子模块让 config.rs 主体专注于目录布局、`ConfigData` 存取
//! 与 bootstrap；`crate::config::` 的既有引用面经 config.rs 的 re-export
//! 零变化。

/// Window-close behavior preference. Crosses the Rust→JS boundary.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum CloseBehavior {
    /// Show the minimize/quit dialog each time (default).
    #[default]
    Ask,
    /// Always minimize to tray — keeps the background scheduler alive.
    Minimize,
    /// Always quit.
    Quit,
}

/// How the lightweight glance card's tucked half-icon expands.
/// Crosses the Rust→JS boundary; Rust itself doesn't act on it (a pure frontend
/// interaction), but it rides `ConfigData` so every Settings preference lives in
/// one place.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum LightweightExpand {
    /// Click the half-icon to expand (default — won't fire on a stray hover).
    #[default]
    Click,
    /// Hover the half-icon to expand.
    Hover,
}

/// Color skin for multi-skin theming (token-first). Serialized
/// snake_case; `neutral` is the default and maps to NO `data-skin` attribute on
/// `<html>` (the :root/.dark values in src/index.css ARE the Neutral palette —
/// pure greyscale chrome over a default multi-hue chart). Per-device, not synced
/// (config.json never enters the repo). The four chromatic skins each override
/// `--brand` (+ `--brand-strong`) and the button-foreground vars in index.css;
/// everything else holds. The frontend applies it; Rust only stores it.
///
/// Back-compat: the legacy snake_case names (`pixso`/`cuiwei`/`tingwu`/
/// `yanzhi`/`zizi`) are accepted as aliases, so an older config.json lands on
/// the closest new skin instead of failing to deserialize — `pixso` (the old
/// default) → `Neutral` (the new default); the rest map by hue family.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum Skin {
    #[default]
    #[serde(alias = "pixso")]
    Neutral,
    #[serde(alias = "cuiwei")]
    Sage,
    #[serde(alias = "tingwu")]
    Azure,
    #[serde(alias = "yanzhi")]
    Crimson,
    #[serde(alias = "zizi")]
    Mauve,
}

/// Display language. Serialized lowercase (`en`/`zh`/`ja`), matching
/// the frontend locale codes. The tray "Quit" item — the only user-facing Rust
/// string — is localized from this; all other UI text is frontend i18n.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Zh,
    Ja,
}
