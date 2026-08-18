//! Dirty-day tracking: the push path's per-day Artifact recompute driver.
//!
//! Stores the day-buckets holding un-pushed local changes and the per-day
//! source rows the push path re-exports. Also hosts the shared
//! `mark_days_dirty` helper used by the collect-side ingest path, and the
//! combined dirty-flag clear ([`Store::clear_dirty_flags_if_unchanged`]) that
//! the push flow uses to drop days AND sessions in one transaction.

use super::schema;
use super::*;

/// Recompute-time row counts for one day — exactly what the push materialized
/// from the store. `clear_dirty_flags_if_unchanged` re-checks these counts
/// before dropping the day's dirty flag, so a row that raced in after the
/// snapshot keeps the day dirty (a blind delete would strand it on the
/// local-only side of git forever).
pub struct DaySnapshot {
    pub day: String,
    pub usage_rows: usize,
    pub turn_rows: usize,
}

impl super::Store {
    // ---------------- Dirty flags (sync recompute driver) ----------------

    /// The day-buckets holding un-pushed local changes, in deterministic order
    /// (sorted). Drives the push path's per-day Artifact recompute. Read-only —
    /// it does NOT clear: clearing happens only after a push lands (see
    /// [`Self::clear_dirty_flags_if_unchanged`]), so a failed push leaves the
    /// days dirty for the next retry. Pure local state: this makes no claim
    /// about the git worktree and never reads it.
    pub fn dirty_days(&self) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare("SELECT day FROM dirty_days ORDER BY day")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Drop the dirty flags for BOTH flag domains — days and sessions — in ONE
    /// transaction, each side scoped to its recompute-time snapshot:
    /// - a day whose `usage_records`/`turn_durations` counts still match the
    ///   snapshot is cleared; a count that grew since keeps the day dirty;
    /// - a session whose message count still matches is cleared; a count that
    ///   grew keeps the session dirty;
    /// - `removed` sessions (non-favorited, snapshot deleted) clear
    ///   unconditionally — deletion is idempotent, and a raced re-favorite
    ///   re-marks the session dirty in `set_session_favorited`.
    ///
    /// The single transaction is the pairing invariant: days and sessions
    /// clear together or not at all. A mid-transaction failure rolls back
    /// everything, so the store can never sit in "days cleared, sessions still
    /// dirty" (or the reverse) — the state a crash between two separate clears
    /// used to leave, which a later no-op push could never heal.
    ///
    /// The per-flag check runs BEFORE its delete in the same transaction, so a
    /// flag can never be dropped after new rows land between the two. Row
    /// counts suffice: per-device rows are INSERT-only (a count mismatch
    /// exactly means "new row since the snapshot"); `forget_device` wipes a
    /// whole device, not one flag, so it never hides a mismatch.
    pub fn clear_dirty_flags_if_unchanged(
        &self,
        device_id: &str,
        day_snapshots: &[DaySnapshot],
        recomputed: &[SessionCounts],
        removed: &[String],
    ) -> AppResult<()> {
        if day_snapshots.is_empty() && recomputed.is_empty() && removed.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        for s in day_snapshots {
            let usage: i64 = tx.query_row(
                "SELECT COUNT(*) FROM usage_records WHERE day = ?1 AND device_id = ?2",
                params![s.day, device_id],
                |r| r.get(0),
            )?;
            let turns: i64 = tx.query_row(
                "SELECT COUNT(*) FROM turn_durations WHERE day = ?1 AND device_id = ?2",
                params![s.day, device_id],
                |r| r.get(0),
            )?;
            if usage == s.usage_rows as i64 && turns == s.turn_rows as i64 {
                tx.execute("DELETE FROM dirty_days WHERE day = ?1", params![s.day])?;
            }
        }
        for s in recomputed {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM session_messages WHERE device_id = ?1 AND session_id = ?2",
                params![device_id, s.session_id],
                |r| r.get(0),
            )?;
            if count == s.message_rows as i64 {
                tx.execute(
                    "DELETE FROM dirty_sessions WHERE session_id = ?1",
                    params![s.session_id],
                )?;
            }
        }
        for sid in removed {
            tx.execute(
                "DELETE FROM dirty_sessions WHERE session_id = ?1",
                params![sid],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Every usage row for one (day, device), in uuid order — the source for the
    /// push path's per-day Artifact recompute. `ORDER BY uuid` (not collect
    /// order) is what makes the rewrite byte-stable across pushes: the same
    /// store yields the same file bytes every time, so git sees no churn once a
    /// day is settled.
    pub fn usage_for_day_device(&self, day: &str, device_id: &str) -> AppResult<Vec<UsageRecord>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        // Column list derives from the schema constant (same single source of
        // truth as `ingest_impl`'s INSERT) — column order is the field order
        // `row_to_usage_record` reads positionally.
        let select_sql = format!(
            "SELECT {} FROM usage_records WHERE day = ? AND device_id = ? ORDER BY uuid",
            schema::USAGE_RECORDS_COLNAMES
        );
        let mut stmt = conn.prepare(&select_sql)?;
        let rows = stmt.query_map(params![day, device_id], row_to_usage_record)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Every turn duration for one (day, device), in uuid order — the source for
    /// the turns Artifact recompute. Same byte-stability rationale as
    /// [`Self::usage_for_day_device`].
    pub fn turns_for_day_device(&self, day: &str, device_id: &str) -> AppResult<Vec<TurnDuration>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT uuid, timestamp, day, device_id, duration_ms
             FROM turn_durations WHERE day = ? AND device_id = ? ORDER BY uuid",
        )?;
        let rows = stmt.query_map(params![day, device_id], |r| {
            Ok(TurnDuration {
                uuid: r.get(0)?,
                timestamp: r.get(1)?,
                day: r.get(2)?,
                device_id: r.get(3)?,
                duration_ms: r.get::<_, i64>(4)? as u32,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }
}

/// Reconstruct a full [`UsageRecord`] (with nested token / cost structs) from a
/// `usage_records` row — the inverse of `Store::ingest_impl`'s insert. Used by
/// the push path's per-day recompute to serialize the day's full content.
fn row_to_usage_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    use std::str::FromStr;
    let dec =
        |s: String| rust_decimal::Decimal::from_str(&s).unwrap_or(rust_decimal::Decimal::ZERO);
    let total = dec(r.get::<_, String>(20)?);
    Ok(UsageRecord {
        uuid: r.get(0)?,
        timestamp: r.get(1)?,
        day: r.get(2)?,
        model: r.get(3)?,
        pricing_model: r.get(4)?,
        source: r.get(5)?,
        session_id: r.get(6)?,
        device_id: r.get(7)?,
        tokens: TokenCounts {
            input: r.get::<_, i64>(8)? as u32,
            output: r.get::<_, i64>(9)? as u32,
            cache_creation: r.get::<_, i64>(10)? as u32,
            cache_read: r.get::<_, i64>(11)? as u32,
        },
        server_tool_use: serde_json::from_str(&r.get::<_, String>(12)?)
            .unwrap_or(crate::model::ServerToolUse::default()),
        stop_reason: r.get(13)?,
        service_tier: r.get(14)?,
        iterations: r.get::<_, i64>(15)? as u32,
        cost: crate::model::CostBreakdown {
            input_usd: dec(r.get::<_, String>(16)?),
            output_usd: dec(r.get::<_, String>(17)?),
            cache_read_usd: dec(r.get::<_, String>(18)?),
            cache_creation_usd: dec(r.get::<_, String>(19)?),
            total_usd: total,
        },
    })
}

/// Flag each day in `days` as dirty, within `tx` so the flag lands atomically
/// with the row writes that made them dirty (a separate transaction could leave
/// a written row whose day is never flagged, silently dropping it from the next
/// push). `INSERT OR IGNORE` keeps it idempotent across collects.
pub(super) fn mark_days_dirty(
    tx: &rusqlite::Transaction,
    days: &std::collections::BTreeSet<String>,
) -> AppResult<()> {
    if days.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare("INSERT OR IGNORE INTO dirty_days(day) VALUES (?1)")?;
    for day in days {
        stmt.execute(params![day])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;

    // ---- dirty_days (sync recompute driver) ----

    /// The local-collect ingest flags each newly inserted row's day dirty, in
    /// the same transaction as the write. A collect that ingests rows on D1 and
    /// D2 leaves exactly {D1, D2} dirty (deduped, sorted).
    #[test]
    fn ingest_marking_dirty_flags_days_of_new_rows() {
        let s = mem();
        s.ingest_marking_dirty(&[
            rec("a", "2026-07-13", "glm-5.2", "dev1", 100, 50, 1.0),
            rec("b", "2026-07-14", "glm-5.2", "dev1", 200, 0, 2.0),
            rec("c", "2026-07-13", "gpt-4o", "dev1", 10, 0, 0.0),
        ])
        .unwrap();
        assert_eq!(
            s.dirty_days().unwrap(),
            vec!["2026-07-13".to_string(), "2026-07-14".to_string()],
            "D1 (two rows) + D2, deduped and sorted"
        );
    }

    /// The pull ingest path must NOT flag days dirty — imported rows are already
    /// on git, so flagging them would only cause spurious recomputes and muddy
    /// the "local dirtiness" invariant.
    #[test]
    fn pull_ingest_does_not_flag_days_dirty() {
        let s = mem();
        s.ingest(&[rec("a", "2026-07-13", "glm-5.2", "dev1", 100, 50, 1.0)])
            .unwrap();
        assert!(
            s.dirty_days().unwrap().is_empty(),
            "pull-path ingest must not flag days dirty"
        );
    }

    /// Re-ingesting already-known rows (uuid dedup) writes nothing new, so it
    /// must not flag any day dirty — otherwise a retried collect would re-dirty
    /// settled days forever. (Clearing first proves the second ingest adds nil.)
    #[test]
    fn deduped_reingest_does_not_flag_dirty() {
        let s = mem();
        let r = rec("a", "2026-07-13", "glm-5.2", "dev1", 100, 50, 1.0);
        s.ingest_marking_dirty(std::slice::from_ref(&r)).unwrap();
        // Snapshot (1 usage row, 0 turns) still matches ⇒ cleared.
        s.clear_dirty_flags_if_unchanged(
            "dev1",
            &[DaySnapshot {
                day: "2026-07-13".into(),
                usage_rows: 1,
                turn_rows: 0,
            }],
            &[],
            &[],
        )
        .unwrap();
        // Same uuid again ⇒ no new row ⇒ no dirty flag.
        s.ingest_marking_dirty(std::slice::from_ref(&r)).unwrap();
        assert!(s.dirty_days().unwrap().is_empty());
    }

    /// Regression (review): a day whose store grew between the recompute
    /// snapshot and the clear must NOT be cleared — the new row has not reached
    /// git, and clearing would strand it forever. The blind
    /// `DELETE WHERE day IN (...)` this snapshot API replaces could not tell
    /// the two apart.
    #[test]
    fn clear_keeps_day_dirty_when_rows_grew_since_snapshot() {
        let s = mem();
        s.ingest_marking_dirty(std::slice::from_ref(&rec(
            "a",
            "2026-07-13",
            "glm-5.2",
            "dev1",
            100,
            50,
            1.0,
        )))
        .unwrap();
        // Snapshot taken at recompute time: 1 usage row for the day. A
        // concurrent collect lands a second row for the SAME day before the
        // push's clear runs.
        s.ingest_marking_dirty(std::slice::from_ref(&rec(
            "b",
            "2026-07-13",
            "glm-5.2",
            "dev1",
            10,
            20,
            2.0,
        )))
        .unwrap();
        s.clear_dirty_flags_if_unchanged(
            "dev1",
            &[DaySnapshot {
                day: "2026-07-13".into(),
                usage_rows: 1,
                turn_rows: 0,
            }],
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            s.dirty_days().unwrap(),
            vec!["2026-07-13".to_string()],
            "day with a post-snapshot row stays dirty"
        );
    }

    /// Turn ingest marks its days dirty on the collect path too (one shared
    /// dirty_days set serves both grains).
    #[test]
    fn turn_ingest_marking_dirty_flags_days() {
        let s = mem();
        s.ingest_turn_durations_marking_dirty(&[
            TurnDuration {
                uuid: "t1".into(),
                timestamp: "2026-07-13T10:00:00Z".into(),
                day: "2026-07-13".into(),
                device_id: "d".into(),
                duration_ms: 100_000,
            },
            TurnDuration {
                uuid: "t2".into(),
                timestamp: "2026-07-14T11:00:00Z".into(),
                day: "2026-07-14".into(),
                device_id: "d".into(),
                duration_ms: 200_000,
            },
        ])
        .unwrap();
        assert_eq!(
            s.dirty_days().unwrap(),
            vec!["2026-07-13".to_string(), "2026-07-14".to_string()]
        );
        // Pull-path turn ingest does not flag. (Snapshots still match — the
        // turn count per day is 1.)
        s.clear_dirty_flags_if_unchanged(
            "d",
            &[
                DaySnapshot {
                    day: "2026-07-13".into(),
                    usage_rows: 0,
                    turn_rows: 1,
                },
                DaySnapshot {
                    day: "2026-07-14".into(),
                    usage_rows: 0,
                    turn_rows: 1,
                },
            ],
            &[],
            &[],
        )
        .unwrap();
        s.ingest_turn_durations(&[TurnDuration {
            uuid: "t3".into(),
            timestamp: "2026-07-15T10:00:00Z".into(),
            day: "2026-07-15".into(),
            device_id: "d".into(),
            duration_ms: 1,
        }])
        .unwrap();
        assert!(
            s.dirty_days().unwrap().is_empty(),
            "pull turn ingest no flag"
        );
    }

    /// dirty_days accumulates across separate collects (a day stays dirty until
    /// the push path clears it).
    #[test]
    fn dirty_days_accumulate_across_collects() {
        let s = mem();
        s.ingest_marking_dirty(&[rec("a", "2026-07-13", "glm-5.2", "d", 1, 0, 0.0)])
            .unwrap();
        s.ingest_marking_dirty(&[rec("b", "2026-07-14", "glm-5.2", "d", 1, 0, 0.0)])
            .unwrap();
        assert_eq!(
            s.dirty_days().unwrap(),
            vec!["2026-07-13".to_string(), "2026-07-14".to_string()]
        );
    }

    // ---- combined clear: days + sessions clear together or not at all ----

    /// The production clear shape: ONE call drops a matching day AND a matching
    /// session together, and the per-flag if-unchanged guard still applies
    /// within the same call — a day whose rows grew since the snapshot stays
    /// dirty while the untouched session clears.
    #[test]
    fn clear_flags_if_unchanged_clears_days_and_sessions_together() {
        let s = mem();
        let dev = "dev1";
        // Day with one usage row + session with one message, both dirty.
        s.ingest_marking_dirty(&[rec("u1", "2026-07-13", "glm-5.2", dev, 1, 0, 0.0)])
            .unwrap();
        seed_session(&s, "s1", dev, "2026-08-15T10:00:00.000Z");
        s.ingest_session_messages_marking_dirty(
            dev,
            &[msg("m1", "s1", SessionMessageRole::User, "2026-07-13T10:00:00Z")],
        )
        .unwrap();
        assert_eq!(s.dirty_days().unwrap().len(), 1);
        assert_eq!(s.dirty_sessions().unwrap(), vec!["s1".to_string()]);

        // Matching snapshots ⇒ both clear in one call.
        s.clear_dirty_flags_if_unchanged(
            dev,
            &[DaySnapshot {
                day: "2026-07-13".into(),
                usage_rows: 1,
                turn_rows: 0,
            }],
            &[SessionCounts {
                session_id: "s1".into(),
                message_rows: 1,
            }],
            &[],
        )
        .unwrap();
        assert!(s.dirty_days().unwrap().is_empty());
        assert!(s.dirty_sessions().unwrap().is_empty());

        // Both grow a post-snapshot row ⇒ both stay dirty (the raced-write
        // guard applies per flag inside the same transaction).
        s.ingest_marking_dirty(&[rec("u2", "2026-07-13", "glm-5.2", dev, 1, 0, 0.0)])
            .unwrap();
        s.ingest_session_messages_marking_dirty(
            dev,
            &[msg("m2", "s1", SessionMessageRole::User, "2026-07-13T10:00:01Z")],
        )
        .unwrap();
        s.clear_dirty_flags_if_unchanged(
            dev,
            &[DaySnapshot {
                day: "2026-07-13".into(),
                usage_rows: 1,
                turn_rows: 0,
            }],
            &[SessionCounts {
                session_id: "s1".into(),
                message_rows: 1,
            }],
            &[],
        )
        .unwrap();
        assert_eq!(s.dirty_days().unwrap(), vec!["2026-07-13".to_string()]);
        assert_eq!(s.dirty_sessions().unwrap(), vec!["s1".to_string()]);
    }

    /// The clear's pairing invariant under failure: days and sessions clear in
    /// ONE transaction, so a mid-transaction failure (here: another connection
    /// holding the SQLite write lock turns the clear's first write into
    /// SQLITE_BUSY) rolls back EVERYTHING — the store is left with both flag
    /// sets dirty, never days-clean + sessions-dirty. The failure is injected
    /// on the SESSIONS side (the day's snapshot mismatches, so the days
    /// section only reads; the sessions DELETE is the clear's first write) —
    /// the "second clear fails" scenario the old two-transaction clear got
    /// wrong. Seeding and the failing call run the production paths
    /// (`ingest_marking_dirty` / `ingest_session_messages_marking_dirty` /
    /// `clear_dirty_flags_if_unchanged`) on a real file-backed store.
    #[test]
    fn clear_flags_if_unchanged_rolls_back_both_domains_on_mid_transaction_failure() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cc-one-test.db");
        let s = Store::open(&db_path).unwrap();
        let dev = "dev1";
        // Day + session, both dirty (1 row each at snapshot time).
        s.ingest_marking_dirty(&[rec("u1", "2026-07-13", "glm-5.2", dev, 1, 0, 0.0)])
            .unwrap();
        seed_session(&s, "s1", dev, "2026-08-15T10:00:00.000Z");
        s.ingest_session_messages_marking_dirty(
            dev,
            &[msg("m1", "s1", SessionMessageRole::User, "2026-07-13T10:00:00Z")],
        )
        .unwrap();
        // A second usage row lands AFTER the snapshot ⇒ the day's count
        // mismatches and the days section performs no writes.
        s.ingest_marking_dirty(&[rec("u2", "2026-07-13", "glm-5.2", dev, 1, 0, 0.0)])
            .unwrap();
        let day_snapshots = [DaySnapshot {
            day: "2026-07-13".into(),
            usage_rows: 1,
            turn_rows: 0,
        }];
        let recomputed = [SessionCounts {
            session_id: "s1".into(),
            message_rows: 1,
        }];

        // A second connection to the same file holds the write lock
        // (BEGIN IMMEDIATE). The clear's first WRITE — the sessions DELETE —
        // now fails with SQLITE_BUSY mid-transaction.
        let locker = rusqlite::Connection::open(&db_path).unwrap();
        locker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let err = s
            .clear_dirty_flags_if_unchanged(dev, &day_snapshots, &recomputed, &[])
            .unwrap_err();
        assert!(
            err.to_string().contains("locked"),
            "failure channel is the injected write-lock: {err}"
        );
        drop(locker);

        // Either both cleared or both dirty — a mid-clear failure must leave
        // both dirty (the transaction rolled back), never a split state.
        assert_eq!(s.dirty_days().unwrap(), vec!["2026-07-13".to_string()]);
        assert_eq!(s.dirty_sessions().unwrap(), vec!["s1".to_string()]);
    }
}
