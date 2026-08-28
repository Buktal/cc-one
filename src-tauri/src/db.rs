//! SQLite Local Store.
//!
//! Owns the schema (usage / turns / pricing / device / scan cursors / dirty
//! days), pricing table and device registry. Exposes typed read methods (stats
//! / trend / logs / models) and write methods (ingest, pricing CRUD, rebill) —
//! the JS layer never sees SQL (typed command boundary).
//!
//! Cost columns are `rust_decimal::Decimal` stored as TEXT; sums over
//! them read back as REAL for display (f64 is display-only — JS never recomputes
//! cost).
//!
//! The store is split by domain across `db/*.rs`: each domain file holds its
//! own `impl super::Store` block plus its helpers and tests. Store-method
//! blocks are named `store_*` so they never collide with the same-named
//! top-level domain modules (ingest / devices / sessions / pricing); schema,
//! migrate and testutil are infrastructure, not method blocks. This file keeps
//! only the `Store` type, the `open` constructor, and the module wiring.

mod aggregate_sql;
mod filter_sql;
mod migrate;
mod schema;
mod store_devices;
mod store_dimensions;
mod store_dirty_days;
mod store_groups;
mod store_ingest;
mod store_pricing;
mod store_providers;
mod store_reads;
// The `sessions` domain is split by ownership: the table's write path + sync
// coupling (`store_sessions_writes`), its read path + the shared
// SessionFilter→SQL clause builder (`store_sessions_reads`), and the
// `session_messages` transcript body (`store_transcript`).
mod store_sessions_reads;
mod store_sessions_writes;
mod store_transcript;
mod usage_records_io;

#[cfg(test)]
pub(crate) mod testutil;

pub use store_dirty_days::DaySnapshot;
pub use store_reads::DistinctColumn;
pub use store_sessions_writes::SessionCounts;

use std::sync::Mutex;

use rusqlite::{params, params_from_iter, types::Value as SqlValue, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};
use crate::model::{
    project_identity, DeviceUsageRow, GroupTrack, LocalGroup, LogCostBreakdown, LogsQuery,
    ModelStatsRow, PricingEntry, ProjectCandidates, ProjectStatsRow, ProjectUsageRow,
    ServerToolUse, SessionFilter, SessionGroupCount, SessionGroupCounts, SessionKey,
    SessionMessage, SessionMessageRole, SessionModelTokens, SessionQuery, SessionRow,
    SessionSnapshotMeta, SessionStatsRow, SessionSystemData, SessionUsageRow, TokenCounts,
    TrendBucket, TrendPoint, TurnDuration, UsageFilter, UsageLogRow, UsageRecord, UsageStats,
    SESSION_SNAPSHOT_VERSION, UNKNOWN_PROJECT,
};
use crate::pricing::{ModelPricing, PricingBook};
use crate::source_parser::{FileCursor, ScanProgress, ScanProgressDelta};

/// 分页 limit 的唯一夹紧点：所有走 SQL `LIMIT` 的分页查询（请求日志
/// `query_logs`、会话列表 `query_sessions_page`）统一经此规范化到
/// [1, 1000]——越界值（0 / 超大）不得穿透（0 会错翻整页、超大一次物化全表；
/// 前端 paginate 曾两份分叉、一份漏夹紧致页码越界——Rust 层不重演两份实现
/// 各改各的）。
pub(crate) fn page_limit(limit: u32) -> i64 {
    limit.clamp(1, 1000) as i64
}

/// Thread-safe wrapper over a single SQLite connection.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) `cc-one.db` and ensure the schema + seed pricing.
    pub fn open(path: &std::path::Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        register_project_identity_udf(&conn)?;
        // Tables → migrate → indexes, in that order. A legacy DB's usage_records
        // predates the session_id column, so idx_usage_session must not run until
        // migrate_schema has ALTERed the column on — building it first panics on
        // upgrade ("no such column: session_id"). The fresh-DB path is unaffected:
        // schema_tables_sql creates every table at its final column set already.
        conn.execute_batch(&schema::schema_tables_sql())?;
        migrate::migrate_schema(&conn)?;
        conn.execute_batch(&schema::schema_indexes_sql())?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.ensure_pricing_seed()?;
        Ok(store)
    }
}

/// Register the `project_identity(text) -> text` SQL scalar on a connection:
/// the single source of the project-dimension bucketing rule, callable from
/// SQL as `project_identity(s.project_dir)` so the project aggregate
/// (`GROUP BY`), the session-list project filter, and the usage-side project
/// filter all bucket by the ONE Rust implementation ([`project_identity`], the
/// #84 rule: a Claude Code `.claude\worktrees\` suffix collapses to its parent
/// project) instead of carrying a second, drifting SQL transcription of it.
/// DETERMINISTIC: same input ⇒ same bucket, safe for GROUP BY and WHERE.
fn register_project_identity_udf(conn: &Connection) -> AppResult<()> {
    use rusqlite::functions::FunctionFlags;
    conn.create_scalar_function(
        "project_identity",
        1,
        FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let raw: String = ctx.get(0)?;
            Ok(project_identity(&raw).to_string())
        },
    )?;
    Ok(())
}
