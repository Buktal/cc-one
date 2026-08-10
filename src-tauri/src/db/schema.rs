//! Single source of truth for the SQLite Local Store schema.
//!
//! Every table's column DDL lives here as a Rust constant. [`schema_tables_sql`]
//! assembles the `CREATE TABLE IF NOT EXISTS ...` batch that [`Store::open`]
//! runs on every open, and the per-table column-name lists feed
//! `migrate_to_composite_pk`'s rebuild. **Adding (or dropping) a column is a
//! single edit here** — there is no parallel `.sql` file to keep in sync
//! (`db_schema.sql` was retired when this module became the source of truth;
//! the human-readable documentation lives in these doc comments).
//!
//! [`Store::open`]: super::Store::open

// ---- Tables that migrate_to_composite_pk rebuilds (uuid → uuid + device_id) ----
//
// Each of these exposes two constants:
//   * `<T>_COLS_DDL`    — column definitions incl. `PRIMARY KEY (...)`, used as
//                        `rebuild_table_pk`'s `new_ddl`.
//   * `<T>_COLNAMES`    — comma-separated column list, used as `rebuild_table_pk`'s
//                        `columns` (the `INSERT...SELECT` projection).
// The [`rebuild_constants_in_sync`] test pins the two together so a column added
// to one cannot silently miss the other.

/// `usage_records` — per-request usage detail (per API request).
///
/// `(uuid, device_id)` = dedup key: the same source event replayed on two
/// devices (the same `~/.claude/projects` scanned under two device ids, or a
/// restored `opencode.db`) yields the same uuid but must be counted per device,
/// never collapsed into one row. `timestamp` is ISO8601 UTC; `day` is the
/// yyyy-mm-dd UTC bucket; `pricing_model` is the rebill lookup key;
/// `server_tool_use` is JSON `{web_search, web_fetch}`; the cost columns are
/// `rust_decimal::Decimal` stored as TEXT.
pub(super) const USAGE_RECORDS_COLS_DDL: &str = "\
    uuid TEXT NOT NULL, \
    timestamp TEXT NOT NULL, \
    day TEXT NOT NULL, \
    model TEXT NOT NULL, \
    pricing_model TEXT NOT NULL, \
    source TEXT NOT NULL, \
    session_id TEXT NOT NULL DEFAULT '', \
    device_id TEXT NOT NULL, \
    input_tokens INTEGER NOT NULL, \
    output_tokens INTEGER NOT NULL, \
    cache_creation_tokens INTEGER NOT NULL, \
    cache_read_tokens INTEGER NOT NULL, \
    server_tool_use TEXT NOT NULL DEFAULT '{}', \
    stop_reason TEXT NOT NULL DEFAULT '', \
    service_tier TEXT NOT NULL DEFAULT '', \
    iterations INTEGER NOT NULL DEFAULT 0, \
    input_cost_usd TEXT NOT NULL, \
    output_cost_usd TEXT NOT NULL, \
    cache_read_cost_usd TEXT NOT NULL, \
    cache_creation_cost_usd TEXT NOT NULL, \
    total_cost_usd TEXT NOT NULL, \
    PRIMARY KEY (uuid, device_id)";

pub(super) const USAGE_RECORDS_COLNAMES: &str = "\
    uuid, timestamp, day, model, pricing_model, source, session_id, device_id, \
    input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, \
    server_tool_use, stop_reason, service_tier, iterations, \
    input_cost_usd, output_cost_usd, cache_read_cost_usd, \
    cache_creation_cost_usd, total_cost_usd";

pub(super) const USAGE_RECORDS_INDEXES: &str = "\
    CREATE INDEX IF NOT EXISTS idx_usage_day ON usage_records(day); \
    CREATE INDEX IF NOT EXISTS idx_usage_model ON usage_records(model); \
    CREATE INDEX IF NOT EXISTS idx_usage_device ON usage_records(device_id); \
    CREATE INDEX IF NOT EXISTS idx_usage_source ON usage_records(source); \
    CREATE INDEX IF NOT EXISTS idx_usage_ts ON usage_records(timestamp); \
    CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_records(session_id);";

/// `turn_durations` — per-turn durations (per-turn grain, separate from per-call
/// `usage_records`). Sourced from system/turn_duration events.
/// `(uuid, device_id)` = dedup key.
pub(super) const TURN_DURATIONS_COLS_DDL: &str = "\
    uuid TEXT NOT NULL, \
    timestamp TEXT NOT NULL, \
    day TEXT NOT NULL, \
    device_id TEXT NOT NULL, \
    duration_ms INTEGER NOT NULL, \
    PRIMARY KEY (uuid, device_id)";

pub(super) const TURN_DURATIONS_COLNAMES: &str = "\
    uuid, timestamp, day, device_id, duration_ms";

pub(super) const TURN_DURATIONS_INDEXES: &str = "\
    CREATE INDEX IF NOT EXISTS idx_turndur_day ON turn_durations(day); \
    CREATE INDEX IF NOT EXISTS idx_turndur_device ON turn_durations(device_id);";

// ---- Tables created by SCHEMA only (never rebuilt — no uuid-only history) ----

/// `dirty_days` — day-buckets (`yyyy-mm-dd`) holding un-pushed local changes.
/// The collect path flags a day dirty in the SAME transaction that writes new
/// usage/turn rows for it; the push path recomputes that day's per-day Artifact
/// from the store and clears the flag once the push lands. One shared set
/// serves both grains — a day is dirty if a usage row OR a turn row landed for
/// it. Local-only: never part of the JSONL Artifact, never synced. It describes
/// local write dirtiness and makes no claim about git worktree state, so a pull
/// that rewrites the worktree can never desync it (contrast with a sync cursor,
/// which would). Minimal shape: the day is the whole row.
pub(super) const DIRTY_DAYS_COLS_DDL: &str = "day TEXT PRIMARY KEY";

/// `dirty_sessions` — session ids holding un-pushed message changes. The collect
/// path flags a session dirty in the SAME transaction that writes its new
/// `session_messages` rows; the push path recomputes that session's derived
/// `sessions/<id>.jsonl` snapshot from the store and clears the flag once the
/// push lands. Mirrors `dirty_days` (one shared dirty-channel per grain).
/// Local-only: never part of the JSONL Artifact, never synced. It describes
/// local write dirtiness and makes no claim about git worktree state, so a pull
/// that rewrites the worktree can never desync it.
pub(super) const DIRTY_SESSIONS_COLS_DDL: &str = "session_id TEXT PRIMARY KEY";

/// `model_pricing` — LiteLLM seed + user overrides. Decimal as TEXT.
pub(super) const MODEL_PRICING_COLS_DDL: &str = "\
    model_key TEXT PRIMARY KEY, \
    display_name TEXT NOT NULL, \
    input_per_million TEXT NOT NULL, \
    output_per_million TEXT NOT NULL, \
    cache_read_per_million TEXT NOT NULL, \
    cache_creation_per_million TEXT NOT NULL, \
    is_builtin INTEGER NOT NULL DEFAULT 1, \
    updated_at TEXT NOT NULL";

/// `device` — device registry.
pub(super) const DEVICE_COLS_DDL: &str = "\
    device_id TEXT PRIMARY KEY, \
    display_name TEXT NOT NULL, \
    is_self INTEGER NOT NULL DEFAULT 0, \
    first_seen TEXT NOT NULL";

/// `scan_progress` — incremental scan cursor. Replaceable cache: a lost/truncated
/// row only triggers a full rescan of that file on the next collect — the store's
/// `(uuid, device_id)` dedup (not this table) is the source of truth. NOT part
/// of the JSONL Artifact; local parse-progress state, not authoritative data.
pub(super) const SCAN_PROGRESS_COLS_DDL: &str = "\
    file_path TEXT PRIMARY KEY, \
    last_modified INTEGER NOT NULL, \
    last_line_offset INTEGER NOT NULL DEFAULT 0";

/// Dead tables dropped on every open, so an upgraded Local Store physically
/// sheds them (idempotent: present ⇒ dropped, absent ⇒ no-op; safe to run on
/// every startup, zero migration code). `daily_rollups` was a derived cache
/// nobody read; `ledger` duplicated the `(uuid, device_id)` dedup.
const DEAD_TABLES_DROP: &str = "\
    DROP TABLE IF EXISTS daily_rollups; \
    DROP TABLE IF EXISTS ledger;";

/// `sessions` — one row per (session id, device id). The system-data columns
/// (source / project_dir / title_orig / started_at / last_active_at) are
/// refreshable: every collect re-extracts them from the source log. The user-
/// data columns (custom_title / favorited / synced_group_id / local_group_id)
/// are NEVER overwritten by re-extract — `upsert_session`'s ON CONFLICT clause
/// refreshes only the system-data columns, preserving user edits. This split is
/// the "user data not overwritten by re-extract" invariant, encoded in SQL.
/// `local_group_id` is device-private (never enters git); the session sync
/// shape lands later.
pub(super) const SESSIONS_COLS_DDL: &str = "\
    id TEXT NOT NULL, \
    device_id TEXT NOT NULL, \
    source TEXT NOT NULL DEFAULT '', \
    project_dir TEXT NOT NULL DEFAULT '', \
    title_orig TEXT NOT NULL DEFAULT '', \
    started_at TEXT NOT NULL DEFAULT '', \
    last_active_at TEXT NOT NULL DEFAULT '', \
    custom_title TEXT NOT NULL DEFAULT '', \
    favorited INTEGER NOT NULL DEFAULT 0, \
    synced_group_id TEXT NOT NULL DEFAULT '', \
    local_group_id TEXT NOT NULL DEFAULT '', \
    PRIMARY KEY (id, device_id)";

pub(super) const SESSIONS_INDEXES: &str = "\
    CREATE INDEX IF NOT EXISTS idx_sessions_device ON sessions(device_id); \
    CREATE INDEX IF NOT EXISTS idx_sessions_favorited ON sessions(favorited);";

/// `local_groups` — device-private group names (local track). Never enters
/// git; CRUD is immediate (no network). `position` is the user-ordered sort
/// key within the track (see `store_groups`).
pub(super) const LOCAL_GROUPS_COLS_DDL: &str = "\
    id TEXT PRIMARY KEY, \
    name TEXT NOT NULL, \
    created_at TEXT NOT NULL, \
    position INTEGER NOT NULL DEFAULT 0";

/// `session_messages` — one row per transcript line, for ALL sessions (not just
/// favorited). `(device_id, uuid)` = dedup key: a source event replayed lands
/// once. SQLite is the single source of truth for message 原文; the
/// `sessions/<id>.jsonl` Artifact is a DERIVED snapshot the push path recomputes
/// for favorited sessions only. `role` is the lowercase spelling from
/// `SessionMessageRole::as_str`; `model`/`name` store the empty string for the
/// `None` side of their `Option<String>` (round-trips losslessly — the role
/// decides which is meaningful, never an empty value standing alone).
pub(super) const SESSION_MESSAGES_COLS_DDL: &str = "\
    device_id TEXT NOT NULL, \
    session_id TEXT NOT NULL, \
    uuid TEXT NOT NULL, \
    role TEXT NOT NULL, \
    ts TEXT NOT NULL, \
    model TEXT NOT NULL DEFAULT '', \
    name TEXT NOT NULL DEFAULT '', \
    content TEXT NOT NULL, \
    PRIMARY KEY (device_id, uuid)";

/// The transcript read path resolves a session by `(device_id, session_id)` and
/// orders by `(ts, uuid)` — this index serves it directly.
pub(super) const SESSION_MESSAGES_INDEXES: &str = "\
    CREATE INDEX IF NOT EXISTS idx_session_messages_session \
        ON session_messages(device_id, session_id);";

/// `provider` — user-created providers (供应商). `settings_config` and `meta`
/// are raw JSON *text* (the store round-trips them without parsing); the API
/// key lives inside `settings_config`'s `env` block. `app` is the owning app
/// (`claude` / `codex` / `gemini`) — the merge/dedup key across sync and
/// export/import is `(app, id)`, and `sort_index` is the user-ordered display
/// rank *within one app pool*. `app` defaults to `'claude'` so pre-dimension
/// rows (and legacy tables ALTERed by `migrate_schema`) all land in the
/// Claude pool.
pub(super) const PROVIDERS_COLS_DDL: &str = "\
    id TEXT PRIMARY KEY, \
    name TEXT NOT NULL, \
    website_url TEXT NOT NULL DEFAULT '', \
    category TEXT NOT NULL DEFAULT 'custom', \
    app TEXT NOT NULL DEFAULT 'claude', \
    icon TEXT NOT NULL DEFAULT '', \
    icon_color TEXT NOT NULL DEFAULT '', \
    sort_index INTEGER NOT NULL DEFAULT 0, \
    notes TEXT NOT NULL DEFAULT '', \
    settings_config TEXT NOT NULL DEFAULT '{}', \
    meta TEXT NOT NULL DEFAULT '{}', \
    updated_at TEXT NOT NULL";

/// All `CREATE TABLE IF NOT EXISTS` statements (no indexes). [`Store::open`]
/// runs this FIRST so every table shell exists before
/// [`migrate::migrate_schema`] ALTERs legacy columns onto them — and before any
/// index that references a column migration adds. Concretely:
/// `idx_usage_session` indexes `session_id`, a column legacy DBs lack until
/// migration ALTERs it on; building that index before the column exists panics
/// on upgrade. Indexes land in [`schema_indexes_sql`], after migration.
///
/// [`Store::open`]: super::Store::open
/// [`migrate::migrate_schema`]: super::migrate::migrate_schema
pub(super) fn schema_tables_sql() -> String {
    [
        DEAD_TABLES_DROP.to_string(),
        create_table("usage_records", USAGE_RECORDS_COLS_DDL),
        create_table("turn_durations", TURN_DURATIONS_COLS_DDL),
        create_table("dirty_days", DIRTY_DAYS_COLS_DDL),
        create_table("dirty_sessions", DIRTY_SESSIONS_COLS_DDL),
        create_table("model_pricing", MODEL_PRICING_COLS_DDL),
        create_table("device", DEVICE_COLS_DDL),
        create_table("scan_progress", SCAN_PROGRESS_COLS_DDL),
        create_table("sessions", SESSIONS_COLS_DDL),
        create_table("local_groups", LOCAL_GROUPS_COLS_DDL),
        create_table("session_messages", SESSION_MESSAGES_COLS_DDL),
        create_table("provider", PROVIDERS_COLS_DDL),
    ]
    .join("\n")
}

/// All `CREATE INDEX IF NOT EXISTS` statements. [`Store::open`] runs this AFTER
/// [`migrate::migrate_schema`] so indexes on migration-added columns (e.g.
/// `idx_usage_session` on `session_id`) find the column present on legacy DBs.
///
/// [`Store::open`]: super::Store::open
/// [`migrate::migrate_schema`]: super::migrate::migrate_schema
pub(super) fn schema_indexes_sql() -> String {
    [
        USAGE_RECORDS_INDEXES.to_string(),
        TURN_DURATIONS_INDEXES.to_string(),
        SESSIONS_INDEXES.to_string(),
        SESSION_MESSAGES_INDEXES.to_string(),
    ]
    .join("\n")
}

fn create_table(name: &str, cols_ddl: &str) -> String {
    format!("CREATE TABLE IF NOT EXISTS {name} ({cols_ddl});")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Column names declared in a `<T>_COLS_DDL` (excluding the `PRIMARY KEY`
    /// clause) — used to cross-check against `<T>_COLNAMES`. Splits on the first
    /// `PRIMARY KEY` so its `(uuid, device_id)` comma is not mistaken for a
    /// column boundary.
    fn ddl_column_names(cols_ddl: &str) -> Vec<&str> {
        let cols = cols_ddl.split("PRIMARY KEY").next().unwrap_or(cols_ddl);
        cols.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.split_whitespace().next().unwrap_or(s))
            .collect()
    }

    /// The rebuildable tables must keep their `COLS_DDL` and `COLNAMES` in
    /// lockstep — otherwise `rebuild_table_pk`'s `INSERT...SELECT` would drop or
    /// misalign columns. This is the single-source invariant the old dual
    /// (`db_schema.sql` + migration literals) setup made fragile.
    #[test]
    fn rebuild_constants_in_sync() {
        for (table, cols_ddl, colnames) in [
            (
                "usage_records",
                USAGE_RECORDS_COLS_DDL,
                USAGE_RECORDS_COLNAMES,
            ),
            (
                "turn_durations",
                TURN_DURATIONS_COLS_DDL,
                TURN_DURATIONS_COLNAMES,
            ),
        ] {
            let from_ddl = ddl_column_names(cols_ddl);
            let from_names: Vec<&str> = colnames.split(',').map(str::trim).collect();
            assert_eq!(
                from_ddl, from_names,
                "{table}: COLNAMES drifted from COLS_DDL"
            );
        }
    }

    /// The assembled batch is valid SQL and creates every expected table. Guards
    /// against a typo in any constant silently skipping a table on open. Runs
    /// tables then indexes — the same order [`Store::open`] uses (a fresh DB
    /// already has every column, so the split is invisible here; it matters only
    /// for legacy upgrades, which the migrate tests cover).
    ///
    /// [`Store::open`]: super::Store::open
    #[test]
    fn schema_tables_then_indexes_creates_all_tables() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(&schema_tables_sql()).unwrap();
        conn.execute_batch(&schema_indexes_sql()).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap();
        let tables: std::collections::HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        for expected in [
            "usage_records",
            "turn_durations",
            "dirty_days",
            "dirty_sessions",
            "model_pricing",
            "device",
            "scan_progress",
            "sessions",
            "local_groups",
            "session_messages",
            "provider",
        ] {
            assert!(tables.contains(expected), "schema missing table {expected}");
        }
    }

    /// An upgraded Local Store that still physically holds the dead tables sheds
    /// them on open — the idempotent DROP runs before the CREATEs, so the store
    /// never keeps `daily_rollups` / `ledger` residue.
    #[test]
    fn schema_drops_dead_tables_from_upgraded_db() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE daily_rollups (day TEXT PRIMARY KEY); \
             CREATE TABLE ledger (uuid TEXT PRIMARY KEY);",
        )
        .unwrap();
        conn.execute_batch(&schema_tables_sql()).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
            .unwrap();
        let tables: std::collections::HashSet<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(!tables.contains("daily_rollups"));
        assert!(!tables.contains("ledger"));
    }
}
