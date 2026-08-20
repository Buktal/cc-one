//! Legacy schema migration for the Local Store.
//!
//! Self-contained upgrade path: [`migrate_schema`] is the single entry point
//! [`Store::open`] runs right after the idempotent [`schema_tables_sql`] batch. It
//! carries forward databases whose tables predate columns added after the
//! initial schema, or the `(uuid, device_id)` composite primary key. New
//! databases are created on the final schema by [`schema_tables_sql`] and short-circuit
//! every probe here.
//!
//! The rebuild path reuses the DDL constants in [`schema`], so adding a column
//! is still a single edit there — migration carries no parallel literal.
//!
//! [`Store::open`]: super::Store::open
//! [`schema_tables_sql`]: super::schema::schema_tables_sql
//! [`schema`]: super::schema

use rusqlite::Connection;

use crate::error::AppResult;

use super::schema;

/// Upgrade a pre-existing `usage_records` table with columns added after the
/// initial schema (scorched-rebuild: `stop_reason` / `service_tier` /
/// `iterations`). `CREATE TABLE IF NOT EXISTS` only creates missing tables —
/// it does **not** add columns to an existing one, so an older Local Store
/// must be upgraded in place. SQLite has no `ADD COLUMN IF NOT EXISTS`, so we
/// probe `table_info` and ALTER each gap. `turn_durations` is a brand-new
/// table and is created normally by SCHEMA.
pub(super) fn migrate_schema(conn: &Connection) -> AppResult<()> {
    let mut have = std::collections::HashSet::new();
    {
        let mut stmt = conn.prepare("PRAGMA table_info(usage_records)")?;
        let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
        for n in names {
            have.insert(n?);
        }
    }
    // (column, DDL) — columns added after the initial schema. These are a subset
    // of `schema::USAGE_RECORDS_COLS_DDL`; ALTER carries them on an old install,
    // then `migrate_to_composite_pk` rebuilds the table from the full constant.
    let need: &[(&str, &str)] = &[
        ("stop_reason", "TEXT NOT NULL DEFAULT ''"),
        ("service_tier", "TEXT NOT NULL DEFAULT ''"),
        ("iterations", "INTEGER NOT NULL DEFAULT 0"),
        ("session_id", "TEXT NOT NULL DEFAULT ''"),
    ];
    for &(col, ddl) in need {
        if !have.contains(col) {
            conn.execute(
                &format!("ALTER TABLE usage_records ADD COLUMN {col} {ddl}"),
                [],
            )?;
        }
    }

    // `local_groups.position` (added after the initial schema) — the sort key
    // for the user-ordered group list. Same probe-ALTER pattern as above; old
    // rows get the DEFAULT 0, i.e. sorted first — the legacy alphabetical
    // order this migration replaces. The table itself may not exist on stores
    // older than the groups feature, so gate on sqlite_master (PRAGMA
    // table_info errors on a missing table).
    {
        let has_table: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='local_groups'",
            [],
            |r| r.get(0),
        )?;
        if has_table > 0 {
            let mut have_local: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut stmt = conn.prepare("PRAGMA table_info(local_groups)")?;
            let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for n in names {
                have_local.insert(n?);
            }
            if !have_local.contains("position") {
                conn.execute(
                    "ALTER TABLE local_groups ADD COLUMN position INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
            }
        }
    }

    // `sessions` columns added after the initial schema: `agent_type` (the
    // subagent type tag, "" = main session) and `parent_session_id` (subagent →
    // its main session's id, "" = none) — both system data, defaulting to the
    // top-level/main shape on legacy rows until the next collect refreshes
    // them; `excluded` (the soft-delete marker, user data — legacy rows default
    // to 0 = present). Same probe-ALTER pattern; the table may not exist on
    // stores older than the sessions feature, so gate on sqlite_master.
    {
        let has_table: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sessions'",
            [],
            |r| r.get(0),
        )?;
        if has_table > 0 {
            let mut have_session: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut stmt = conn.prepare("PRAGMA table_info(sessions)")?;
            let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for n in names {
                have_session.insert(n?);
            }
            for (col, ddl) in [
                ("agent_type", "TEXT NOT NULL DEFAULT ''"),
                ("parent_session_id", "TEXT NOT NULL DEFAULT ''"),
                ("excluded", "INTEGER NOT NULL DEFAULT 0"),
            ] {
                if !have_session.contains(col) {
                    conn.execute(&format!("ALTER TABLE sessions ADD COLUMN {col} {ddl}"), [])?;
                }
            }
        }
    }

    // `provider.app`（应用维度，后续批次加入）— 旧库没有该列，存量行靠
    // DEFAULT 'claude' 自动归入 Claude 池，无需逐行回填。与 local_groups
    // 同样的 probe-ALTER：表可能不存在于比供应商功能更老的库上，先查
    // sqlite_master 再 PRAGMA table_info。
    {
        let has_table: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='provider'",
            [],
            |r| r.get(0),
        )?;
        if has_table > 0 {
            let mut have_provider: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut stmt = conn.prepare("PRAGMA table_info(provider)")?;
            let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for n in names {
                have_provider.insert(n?);
            }
            if !have_provider.contains("app") {
                conn.execute(
                    "ALTER TABLE provider ADD COLUMN app TEXT NOT NULL DEFAULT 'claude'",
                    [],
                )?;
            }
        }
    }

    // `turn_durations.session_id`（项目维度补齐时加入）— 旧库没有该列，存量行靠
    // DEFAULT '' 归入未知项目（无会话行可归属）。与 sessions.agent_type 同样的
    // probe-ALTER：表可能不存在于更老的库上（turns 是后来加的表），先查
    // sqlite_master 再 PRAGMA table_info。
    {
        let has_table: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='turn_durations'",
            [],
            |r| r.get(0),
        )?;
        if has_table > 0 {
            let mut have_turn: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut stmt = conn.prepare("PRAGMA table_info(turn_durations)")?;
            let names = stmt.query_map([], |r| r.get::<_, String>(1))?;
            for n in names {
                have_turn.insert(n?);
            }
            if !have_turn.contains("session_id") {
                conn.execute(
                    "ALTER TABLE turn_durations ADD COLUMN session_id TEXT NOT NULL DEFAULT ''",
                    [],
                )?;
            }
        }
    }

    // uuid 单列 PRIMARY KEY → (uuid, device_id) 复合主键。旧库的 usage_records /
    // turn_durations 以 uuid 为单列主键,把"同 uuid、不同设备"的记录折叠成一条
    // (后导入者被丢)——同一份 ~/.claude/projects 被两个 device_id 扫描,或
    // opencode.db 被恢复到第二台机器,都会撞 uuid。重建为复合主键,各设备各自保留。
    // 新库由 SCHEMA 直接建复合主键,这里检测后跳过。
    migrate_to_composite_pk(conn)?;

    Ok(())
}

/// 表存在、但 `device_id` 不在 PRIMARY KEY 里 ⇒ 旧的单列 uuid 主键 schema,需迁移。
fn needs_composite_pk_migration(conn: &Connection, table: &str) -> AppResult<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))?;
    let mut device_pk: Option<i64> = None;
    for r in rows {
        let (name, pk) = r?;
        if name == "device_id" {
            device_pk = Some(pk);
        }
    }
    Ok(device_pk == Some(0))
}

/// SQLite 不能 ALTER 主键,只能重建:建新表(复合主键)→ 拷数据 → 删旧 → 改名。
fn rebuild_table_pk(conn: &Connection, table: &str, new_ddl: &str, columns: &str) -> AppResult<()> {
    let tmp = format!("{table}__migrate");
    conn.execute(&format!("CREATE TABLE {tmp} ({new_ddl})"), [])?;
    conn.execute(
        &format!("INSERT INTO {tmp} ({columns}) SELECT {columns} FROM {table}"),
        [],
    )?;
    conn.execute(&format!("DROP TABLE {table}"), [])?;
    conn.execute(&format!("ALTER TABLE {tmp} RENAME TO {table}"), [])?;
    Ok(())
}

fn migrate_to_composite_pk(conn: &Connection) -> AppResult<()> {
    if needs_composite_pk_migration(conn, "usage_records")? {
        rebuild_table_pk(
            conn,
            "usage_records",
            schema::USAGE_RECORDS_COLS_DDL,
            schema::USAGE_RECORDS_COLNAMES,
        )?;
        conn.execute_batch(schema::USAGE_RECORDS_INDEXES)?;
    }
    if needs_composite_pk_migration(conn, "turn_durations")? {
        rebuild_table_pk(
            conn,
            "turn_durations",
            schema::TURN_DURATIONS_COLS_DDL,
            schema::TURN_DURATIONS_COLNAMES,
        )?;
        conn.execute_batch(schema::TURN_DURATIONS_INDEXES)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_upgrades_legacy_usage_records() {
        // Reproduce a pre-scorched-rebuild Local Store: usage_records without
        // the per-call stop_reason / service_tier / iterations columns.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT PRIMARY KEY, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}',
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL
            );",
        )
        .unwrap();
        // Legacy table lacks the new columns.
        assert!(conn
            .prepare("SELECT stop_reason FROM usage_records")
            .is_err());

        migrate_schema(&conn).unwrap();

        // Columns now present; an insert that omits them gets the defaults.
        conn.execute(
            "INSERT INTO usage_records (uuid, timestamp, day, model, pricing_model, source,
                device_id, input_tokens, output_tokens, cache_creation_tokens,
                cache_read_tokens, server_tool_use, input_cost_usd, output_cost_usd,
                cache_read_cost_usd, cache_creation_cost_usd, total_cost_usd)
             VALUES ('u1','2026-07-21T00:00:00Z','2026-07-21','glm-5.2','glm-5.2',
                'claude_code','dev1',1,2,3,4,'{}','0','0','0','0','0')",
            [],
        )
        .unwrap();
        let (stop, tier, iters): (String, String, i64) = conn
            .query_row(
                "SELECT stop_reason, service_tier, iterations FROM usage_records WHERE uuid='u1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(stop, "");
        assert_eq!(tier, "");
        assert_eq!(iters, 0);
    }

    /// Regression: an existing Local Store on the old single-column uuid PK
    /// schema is migrated in place to (uuid, device_id), and the migrated store
    /// then keeps same-uuid-across-devices rows.
    #[test]
    fn migrate_upgrades_uuid_pk_to_composite() {
        let conn = Connection::open_in_memory().unwrap();
        // Old schema: uuid is the sole PRIMARY KEY.
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT PRIMARY KEY, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}', stop_reason TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '', iterations INTEGER NOT NULL DEFAULT 0,
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL
            );
            CREATE TABLE turn_durations (
                uuid TEXT PRIMARY KEY, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                device_id TEXT NOT NULL, duration_ms INTEGER NOT NULL
            );",
        )
        .unwrap();
        migrate_schema(&conn).unwrap();

        // device_id is now part of the PK on both tables.
        let pk: Vec<(String, i64)> = conn
            .prepare("PRAGMA table_info(usage_records)")
            .unwrap()
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|(n, _)| n == "uuid" || n == "device_id")
            .collect();
        assert!(pk.iter().all(|(_, p)| *p > 0), "uuid+device_id both in PK");

        // A same-uuid/different-device pair now coexists.
        conn.execute(
            "INSERT INTO usage_records (uuid, timestamp, day, model, pricing_model, source,
                device_id, input_tokens, output_tokens, cache_creation_tokens,
                cache_read_tokens, server_tool_use, stop_reason, service_tier, iterations,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd)
             VALUES ('u1','2026-07-30T00:00:00Z','2026-07-30','glm-5.2','glm-5.2','claude_code',
                'aaaaaa000001',1,2,3,4,'{}','','',0,'0','0','0','0','0')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage_records (uuid, timestamp, day, model, pricing_model, source,
                device_id, input_tokens, output_tokens, cache_creation_tokens,
                cache_read_tokens, server_tool_use, stop_reason, service_tier, iterations,
                input_cost_usd, output_cost_usd, cache_read_cost_usd, cache_creation_cost_usd,
                total_cost_usd)
             VALUES ('u1','2026-07-30T00:00:00Z','2026-07-30','glm-5.2','glm-5.2','claude_code',
                'bbbbbb000002',1,2,3,4,'{}','','',0,'0','0','0','0','0')",
            [],
        )
        .unwrap();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM usage_records WHERE uuid='u1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2, "both devices kept after migration");
    }

    /// Regression: a legacy Local Store whose `usage_records` predates the
    /// `session_id` column used to panic on open — the old single `schema_sql()`
    /// batch built `idx_usage_session` before `migrate_schema()` ALTERed the
    /// column on. `Store::open` now runs tables → migrate → indexes; this pins
    /// that order on a legacy DB so an upgrade never panics.
    #[test]
    fn indexes_on_migration_added_columns_run_after_the_alter() {
        let conn = Connection::open_in_memory().unwrap();
        // Legacy usage_records: no session_id, old single-column uuid PK.
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT PRIMARY KEY, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}',
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL
            );",
        )
        .unwrap();
        // The Store::open order on a pre-existing DB: tables (table exists →
        // no-op) → migrate (adds session_id) → indexes (session_id now present).
        conn.execute_batch(&schema::schema_tables_sql()).unwrap();
        migrate_schema(&conn).unwrap();
        conn.execute_batch(&schema::schema_indexes_sql()).unwrap();

        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(usage_records)")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(cols.iter().any(|c| c == "session_id"), "session_id added");

        let idx: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master \
                 WHERE type='index' AND name='idx_usage_session'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1, "idx_usage_session created after the column landed");
    }

    /// Regression: a legacy `local_groups` table without the `position` column
    /// (added with drag-to-reorder) is upgraded in place by probe-ALTER, and
    /// pre-existing rows fall back to position 0.
    #[test]
    fn migrate_adds_local_groups_position() {
        let conn = Connection::open_in_memory().unwrap();
        // usage_records must exist (migrate_schema probes it first) — built
        // with the full current column set so only local_groups is legacy.
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT PRIMARY KEY, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}', stop_reason TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '', iterations INTEGER NOT NULL DEFAULT 0,
                session_id TEXT NOT NULL DEFAULT '',
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL
            );
            CREATE TABLE local_groups (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL
            );
            INSERT INTO local_groups (id, name, created_at)
                VALUES ('g1', 'Legacy', '2026-08-01T00:00:00Z');",
        )
        .unwrap();
        assert!(conn.prepare("SELECT position FROM local_groups").is_err());

        migrate_schema(&conn).unwrap();

        let pos: i64 = conn
            .query_row("SELECT position FROM local_groups WHERE id='g1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pos, 0, "legacy rows default to position 0");
    }

    /// Regression: a legacy `provider` table without the `app` column
    /// (pre-app-dimension) is upgraded in place by probe-ALTER, and
    /// pre-existing rows fall back to 'claude' — the whole pre-dimension pool
    /// lands in Claude without a row-by-row backfill.
    #[test]
    fn migrate_adds_provider_app() {
        let conn = Connection::open_in_memory().unwrap();
        // usage_records must exist (migrate_schema probes it first) — built
        // with the full current column set so only provider is legacy.
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT PRIMARY KEY, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}', stop_reason TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '', iterations INTEGER NOT NULL DEFAULT 0,
                session_id TEXT NOT NULL DEFAULT '',
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL
            );
            CREATE TABLE provider (
                id TEXT PRIMARY KEY, name TEXT NOT NULL, website_url TEXT NOT NULL DEFAULT '',
                category TEXT NOT NULL DEFAULT 'custom', icon TEXT NOT NULL DEFAULT '',
                icon_color TEXT NOT NULL DEFAULT '', sort_index INTEGER NOT NULL DEFAULT 0,
                notes TEXT NOT NULL DEFAULT '', settings_config TEXT NOT NULL DEFAULT '{}',
                meta TEXT NOT NULL DEFAULT '{}', updated_at TEXT NOT NULL
            );
            INSERT INTO provider (id, name, updated_at)
                VALUES ('p1', 'Legacy', '2026-08-01T00:00:00Z');",
        )
        .unwrap();
        assert!(conn.prepare("SELECT app FROM provider").is_err());

        migrate_schema(&conn).unwrap();

        let app: String = conn
            .query_row("SELECT app FROM provider WHERE id='p1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(app, "claude", "legacy rows fall back to the claude pool");
    }

    /// Regression: a legacy `turn_durations` table predating the `session_id`
    /// column (the project-dimension backfill) is upgraded in place by
    /// probe-ALTER, and pre-existing rows fall back to '' — the unknown-project
    /// bucket, since no session row resolves an empty id.
    #[test]
    fn migrate_adds_turn_durations_session_id() {
        let conn = Connection::open_in_memory().unwrap();
        // Legacy turn_durations: composite PK already, but no session_id.
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT NOT NULL, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                session_id TEXT NOT NULL DEFAULT '',
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}', stop_reason TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '', iterations INTEGER NOT NULL DEFAULT 0,
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL,
                PRIMARY KEY (uuid, device_id)
            );
            CREATE TABLE turn_durations (
                uuid TEXT NOT NULL, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                device_id TEXT NOT NULL, duration_ms INTEGER NOT NULL,
                PRIMARY KEY (uuid, device_id)
            );
            INSERT INTO turn_durations (uuid, timestamp, day, device_id, duration_ms)
                VALUES ('t1','2026-08-01T00:00:00Z','2026-08-01','dev1',1000);",
        )
        .unwrap();
        assert!(conn
            .prepare("SELECT session_id FROM turn_durations")
            .is_err());

        migrate_schema(&conn).unwrap();

        let sid: String = conn
            .query_row(
                "SELECT session_id FROM turn_durations WHERE uuid='t1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sid, "", "legacy rows default to '' (unknown project)");
    }

    /// Regression: a legacy `sessions` table predating `parent_session_id`
    /// (subagent parent link) and `excluded` (soft-delete marker) is upgraded
    /// in place by probe-ALTER — legacy rows default to `""` (top-level) and
    /// `0` (present) respectively.
    #[test]
    fn migrate_adds_sessions_parent_link_and_excluded() {
        let conn = Connection::open_in_memory().unwrap();
        // usage_records must exist (migrate_schema probes it first) — built
        // with the full current column set so only sessions is legacy.
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT NOT NULL, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                session_id TEXT NOT NULL DEFAULT '', device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}', stop_reason TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '', iterations INTEGER NOT NULL DEFAULT 0,
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL,
                PRIMARY KEY (uuid, device_id)
            );
            CREATE TABLE sessions (
                id TEXT NOT NULL, device_id TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT '', project_dir TEXT NOT NULL DEFAULT '',
                title_orig TEXT NOT NULL DEFAULT '', started_at TEXT NOT NULL DEFAULT '',
                last_active_at TEXT NOT NULL DEFAULT '', agent_type TEXT NOT NULL DEFAULT '',
                custom_title TEXT NOT NULL DEFAULT '', favorited INTEGER NOT NULL DEFAULT 0,
                synced_group_id TEXT NOT NULL DEFAULT '', local_group_id TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (id, device_id)
            );
            INSERT INTO sessions (id, device_id) VALUES ('s1', 'dev1');",
        )
        .unwrap();
        assert!(conn
            .prepare("SELECT parent_session_id FROM sessions")
            .is_err());
        assert!(conn.prepare("SELECT excluded FROM sessions").is_err());

        migrate_schema(&conn).unwrap();

        let (parent, excluded): (String, i64) = conn
            .query_row(
                "SELECT parent_session_id, excluded FROM sessions WHERE id='s1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(parent, "", "legacy rows default to top-level");
        assert_eq!(excluded, 0, "legacy rows default to present");
    }

    /// Regression companion: a legacy DB ALREADY on the composite (uuid,
    /// device_id) PK but predating session_id — the realistic upgrade state for
    /// anyone on a recent cc one before this column shipped. Only the ALTER
    /// runs (no PK rebuild); the index then finds the column present.
    #[test]
    fn alter_adds_session_id_on_already_composite_pk() {
        let conn = Connection::open_in_memory().unwrap();
        // Composite PK, but no session_id (the column this change introduced).
        conn.execute_batch(
            "CREATE TABLE usage_records (
                uuid TEXT NOT NULL, timestamp TEXT NOT NULL, day TEXT NOT NULL,
                model TEXT NOT NULL, pricing_model TEXT NOT NULL, source TEXT NOT NULL,
                device_id TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cache_creation_tokens INTEGER NOT NULL, cache_read_tokens INTEGER NOT NULL,
                server_tool_use TEXT NOT NULL DEFAULT '{}', stop_reason TEXT NOT NULL DEFAULT '',
                service_tier TEXT NOT NULL DEFAULT '', iterations INTEGER NOT NULL DEFAULT 0,
                input_cost_usd TEXT NOT NULL, output_cost_usd TEXT NOT NULL,
                cache_read_cost_usd TEXT NOT NULL, cache_creation_cost_usd TEXT NOT NULL,
                total_cost_usd TEXT NOT NULL,
                PRIMARY KEY (uuid, device_id)
            );",
        )
        .unwrap();
        conn.execute_batch(&schema::schema_tables_sql()).unwrap();
        migrate_schema(&conn).unwrap();
        conn.execute_batch(&schema::schema_indexes_sql()).unwrap();

        let has_session: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('usage_records') \
                 WHERE name='session_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_session, 1, "session_id ALTERed onto composite-PK table");
    }
}
