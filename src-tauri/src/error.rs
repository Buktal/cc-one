//! Typed error channel.
//!
//! Every Tauri command returns `Result<T, AppError>`. `AppError` derives
//! `specta::Type` and is serialized as a tagged enum, so the frontend receives
//! a discriminated union it can narrow on (`{ type: "Db", data: "..." }`).

/// The single error type crossing the Rust→JS boundary.
///
/// Variants are kept coarse and serializable-friendly: low-level causes are
/// stringified into the matching coarse variant (`Io` / `Db` / `Sync`) rather
/// than leaked across the boundary, so the contract stays stable and
/// specta-friendly.
#[derive(Debug, thiserror::Error, serde::Serialize, specta::Type)]
#[serde(tag = "type", content = "data")]
pub enum AppError {
    /// The local data dir / config could not be created or read.
    #[error("config error: {0}")]
    Config(String),
    /// SQLite Local Store error.
    #[error("db error: {0}")]
    Db(String),
    /// A filesystem / std-io failure (read / write / create / rename / delete)
    /// on an ordinary data file: library file ops, pricing-file writes,
    /// exported-provider writes, snapshot reads, … Kept separate from
    /// [`AppError::Config`] because the frontend keys the user-facing message
    /// off the variant name — an unplugged USB drive during an export is not a
    /// "configuration error". `Config` keeps meaning bad config *content* or
    /// unresolvable config identity, not a failing disk operation.
    #[error("io error: {0}")]
    Io(String),
    /// A parser failed to discover/parse Source logs.
    #[error("parser error: {0}")]
    SourceParser(String),
    /// Pricing lookup / cost calc error.
    #[error("pricing error: {0}")]
    Pricing(String),
    /// Sync (git2 / network) error — only raised in Synced mode.
    #[error("sync error: {0}")]
    Sync(String),
    /// 模型列表获取失败（OpenAI 兼容 GET /v1/models，前端「获取模型列表」
    /// 按钮）。串内带稳定前缀标签（AUTH_FAILED / ENDPOINT_CLOSED / TIMEOUT /
    /// BAD_FORMAT / NETWORK），前端按标签分桶成对应的 toast 提示——分桶
    /// 契约见 `provider::model_fetch` 的模块文档。
    #[error("model fetch failed: {0}")]
    FetchModels(String),
    /// Catch-all for anything not covered above.
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Internal(format!("serde: {e}"))
    }
}

impl From<rust_decimal::Error> for AppError {
    fn from(e: rust_decimal::Error) -> Self {
        Self::Pricing(e.to_string())
    }
}

impl From<git2::Error> for AppError {
    fn from(e: git2::Error) -> Self {
        Self::Sync(e.message().to_string())
    }
}

/// `Result` alias used throughout the backend.
pub type AppResult<T> = Result<T, AppError>;
