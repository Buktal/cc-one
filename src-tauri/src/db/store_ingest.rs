//! Usage-record / turn-duration ingest + incremental scan cursors.

use super::schema;
use super::store_dirty_days::mark_days_dirty;
use super::*;

impl super::Store {
    // ---------------- Ingest ----------------

    /// Insert a batch of records, deduping by the `(uuid, device_id)` primary
    /// key (`ON CONFLICT DO NOTHING`). Returns the newly imported rows (in
    /// order). The pull path: imported rows are already on git, so their days
    /// are NOT flagged dirty. The local-collect path uses
    /// [`Store::ingest_marking_dirty`].
    pub fn ingest(&self, records: &[UsageRecord]) -> AppResult<Vec<UsageRecord>> {
        self.ingest_impl(records, false)
    }

    /// Local-collect ingest for NEW events: like [`Store::ingest`], but flags
    /// each inserted row's day dirty in the SAME transaction. Same-tx is
    /// load-bearing — if the row write and the dirty flag were separate
    /// transactions, a crash between them would leave a written row whose day
    /// is never flagged, so the next push's per-day recompute would never pick
    /// it up and it would silently miss git (the exact failure the old
    /// JSONL-first ordering guarded). Pull does not call this: peer rows are
    /// already on git, so flagging their days would only cause spurious
    /// recomputes and muddy the "local dirtiness" invariant (`dirty_days`
    /// describes un-pushed LOCAL writes, never imports).
    ///
    /// The upsert guard ([`collect_guarded_upsert_sql`]) is the re-scan
    /// self-heal backstop here: a lost cursor makes the parser re-emit rows an
    /// earlier pass already wrote, and a conflicting row is rewritten ONLY if
    /// it still reads `model = 'unknown'` — known-model rows are never touched.
    pub fn ingest_marking_dirty(&self, records: &[UsageRecord]) -> AppResult<Vec<UsageRecord>> {
        self.ingest_impl(records, true)
    }

    /// Corrections ingest — the unknown-model self-heal protocol's STORE HALF
    /// (the parser half is `CollectResult::corrections`): a row the Codex
    /// parser re-emits with a now-resolved model (original uuid, model was
    /// "unknown" when an earlier pass wrote it) rewrites the store row ONLY IF
    /// that row still reads `model = 'unknown'` — model + pricing_model + the
    /// cost columns, which all derive from the model — and flags its day dirty
    /// in the same transaction so the push path recomputes the derived
    /// artifact. Rows that already carry the model are never touched: the
    /// parser re-offers corrections on every pass (it cannot tell which
    /// pre-model rows an earlier pass wrote), and this guard turns re-offers
    /// into no-ops. Same guarded SQL + dirty-marking as
    /// [`Store::ingest_marking_dirty`]; separate entry so the protocol half is
    /// independently testable and cannot silently merge back into the events
    /// path.
    pub fn ingest_corrections_marking_dirty(
        &self,
        records: &[UsageRecord],
    ) -> AppResult<Vec<UsageRecord>> {
        self.ingest_impl(records, true)
    }

    fn ingest_impl(
        &self,
        records: &[UsageRecord],
        mark_dirty: bool,
    ) -> AppResult<Vec<UsageRecord>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;

        // Column list and placeholder count derive from the schema constant, so
        // a column added to `schema::USAGE_RECORDS_COLNAMES` cannot silently
        // leave this INSERT stale (single source of truth).
        let cols = schema::USAGE_RECORDS_COLNAMES;
        let placeholders = (1..=cols.split(',').count())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(",");
        let insert_sql = if mark_dirty {
            // The collect path's single guarded upsert, shared by the events
            // ingest (re-scan self-heal backstop) and the corrections ingest
            // (the unknown-model protocol — see both doc comments).
            Self::collect_guarded_upsert_sql(cols, &placeholders)
        } else {
            // Pull path: imported rows are already on git; re-importing must
            // never modify an existing row (pure dedup).
            format!(
                "INSERT INTO usage_records ({cols}) VALUES ({placeholders})
                 ON CONFLICT (uuid, device_id) DO NOTHING
                 RETURNING uuid"
            )
        };

        // Dedup is the `(uuid, device_id)` primary key itself: RETURNING tells
        // us exactly which rows actually landed — inserted, plus (collect path)
        // rows rewritten from "unknown" — so `rows_inserted` and the dirty-day
        // set reflect real changes, not a pre-check that can drift from the
        // table. Device-scoped — the same source event replayed on two devices
        // must be counted per device.
        let mut inserted: Vec<UsageRecord> = Vec::new();
        for r in records {
            let landed: Option<String> = tx
                .query_row(
                    &insert_sql,
                    params![
                        r.uuid,
                        r.timestamp,
                        r.day,
                        r.model,
                        r.pricing_model,
                        r.source,
                        r.session_id,
                        r.device_id,
                        r.tokens.input as i64,
                        r.tokens.output as i64,
                        r.tokens.cache_creation as i64,
                        r.tokens.cache_read as i64,
                        serde_json::to_string(&r.server_tool_use).unwrap_or_else(|_| "{}".into()),
                        r.stop_reason,
                        r.service_tier,
                        r.iterations as i64,
                        r.cost.input_usd.to_string(),
                        r.cost.output_usd.to_string(),
                        r.cost.cache_read_usd.to_string(),
                        r.cost.cache_creation_usd.to_string(),
                        r.cost.total_usd.to_string(),
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if landed.is_some() {
                inserted.push(r.clone());
            }
        }

        if mark_dirty {
            let dirty: std::collections::BTreeSet<String> =
                inserted.iter().map(|r| r.day.clone()).collect();
            mark_days_dirty(&tx, &dirty)?;
        }

        tx.commit()?;
        Ok(inserted)
    }

    /// The collect path's guarded upsert SQL: a conflicting `(uuid, device_id)`
    /// row is rewritten ONLY when it still reads `model = 'unknown'` — model +
    /// pricing_model + the cost columns, which all derive from the model —
    /// because the Codex parser can re-emit a row its earlier pass wrote with
    /// model "unknown" (the log's model context can follow the token events).
    /// Known-model rows are never touched, so re-offered corrections are
    /// no-ops for rows that already carry the model. The rewritten row's day
    /// is flagged dirty (by the caller, in the same tx) so the push path
    /// recomputes the derived artifact, which still holds the stale row.
    fn collect_guarded_upsert_sql(cols: &str, placeholders: &str) -> String {
        format!(
            "INSERT INTO usage_records ({cols}) VALUES ({placeholders})
             ON CONFLICT (uuid, device_id) DO UPDATE SET
               model = excluded.model,
               pricing_model = excluded.pricing_model,
               input_cost_usd = excluded.input_cost_usd,
               output_cost_usd = excluded.output_cost_usd,
               cache_read_cost_usd = excluded.cache_read_cost_usd,
               cache_creation_cost_usd = excluded.cache_creation_cost_usd,
               total_cost_usd = excluded.total_cost_usd
             WHERE usage_records.model = 'unknown'
             RETURNING uuid"
        )
    }

    /// Insert per-turn durations, deduping by uuid (INSERT OR IGNORE). Separate
    /// grain from per-call usage_records. Returns the newly inserted subset
    /// (mirrors `ingest`). Pull path — does not flag days dirty; see
    /// [`Self::ingest_turn_durations_marking_dirty`].
    pub fn ingest_turn_durations(&self, tds: &[TurnDuration]) -> AppResult<Vec<TurnDuration>> {
        self.ingest_turn_durations_impl(tds, false)
    }

    /// Local-collect ingest for turns: flags each inserted turn's day dirty in
    /// the same transaction (same rationale as [`Self::ingest_marking_dirty`]).
    pub fn ingest_turn_durations_marking_dirty(
        &self,
        tds: &[TurnDuration],
    ) -> AppResult<Vec<TurnDuration>> {
        self.ingest_turn_durations_impl(tds, true)
    }

    fn ingest_turn_durations_impl(
        &self,
        tds: &[TurnDuration],
        mark_dirty: bool,
    ) -> AppResult<Vec<TurnDuration>> {
        if tds.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        let mut inserted = Vec::new();
        for td in tds {
            let n = tx.execute(
                "INSERT OR IGNORE INTO turn_durations
                 (uuid, timestamp, day, session_id, device_id, duration_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    td.uuid,
                    td.timestamp,
                    td.day,
                    td.session_id,
                    td.device_id,
                    td.duration_ms as i64
                ],
            )?;
            if n > 0 {
                inserted.push(td.clone());
            }
        }
        if mark_dirty {
            let dirty: std::collections::BTreeSet<String> =
                inserted.iter().map(|t| t.day.clone()).collect();
            mark_days_dirty(&tx, &dirty)?;
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Load all incremental scan cursors. Empty on a fresh/cleared
    /// DB ⇒ the next collect is a full scan (safe fallback — the store dedups).
    pub fn load_scan_progress(&self) -> AppResult<ScanProgress> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT file_path, last_modified, last_line_offset FROM scan_progress")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                FileCursor {
                    last_modified: r.get::<_, i64>(1)?,
                    last_line_offset: r.get::<_, i64>(2)?,
                },
            ))
        })?;
        let mut map = ScanProgress::new();
        for row in rows {
            let (path, cursor) = row?;
            map.insert(path, cursor);
        }
        Ok(map)
    }

    /// Bulk UPSERT incremental scan cursors. Called AFTER a
    /// successful ingest so the cursor never advances past un-ingested rows.
    pub fn save_scan_progress(&self, delta: &ScanProgressDelta) -> AppResult<()> {
        if delta.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "INSERT INTO scan_progress (file_path, last_modified, last_line_offset)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(file_path) DO UPDATE SET
               last_modified = excluded.last_modified,
               last_line_offset = excluded.last_line_offset",
        )?;
        for (path, cursor) in delta {
            stmt.execute(params![path, cursor.last_modified, cursor.last_line_offset])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
    use crate::model::{CostBreakdown, ServerToolUse};

    #[test]
    fn open_then_ingest_surfaces_stop_reason_end_to_end() {
        // A current-schema store (open runs SCHEMA + migrate) must ingest the
        // new per-call fields and surface them in the log query.
        let s = mem();
        s.ingest(std::slice::from_ref(&rec(
            "m1",
            "2026-07-21",
            "glm-5.2",
            "dev1",
            10,
            0,
            0.0,
        )))
        .unwrap();
        let logs = s
            .query_logs(&LogsQuery {
                filter: UsageFilter::default(),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].stop_reason, "end_turn");
    }

    #[test]
    fn ingest_inserts_then_dedups_same_uuid() {
        let s = mem();
        let r = rec("u1", "2026-07-13", "glm-5.2", "dev1", 100, 50, 1.0);
        assert_eq!(s.ingest(std::slice::from_ref(&r)).unwrap().len(), 1);
        assert_eq!(s.ingest(&[r]).unwrap().len(), 0, "same uuid must dedupe");
    }

    /// Codex can place the model context AFTER the token events, so a collect
    /// pass may write a row with model "unknown" that a later pass can resolve.
    /// The events-path ingest (re-scan self-heal backstop: a lost cursor makes
    /// the parser re-emit rows an earlier pass wrote) must rewrite EXACTLY
    /// those rows (model + pricing_model + cost) when the parser re-emits them
    /// corrected, and flag the corrected row's day dirty so the push path
    /// recomputes the artifact. Rows whose model is already known are never
    /// touched. (The corrections channel — the protocol's primary path — is
    /// covered by [`ingest_corrections_marking_dirty_rewrites_only_unknown_rows`];
    /// both share `collect_guarded_upsert_sql`.)
    #[test]
    fn ingest_marking_dirty_corrects_unknown_model_rows_only() {
        let s = mem();
        let first = rec(
            "codex:thread-v1:t1:1",
            "2026-08-07",
            "unknown",
            "dev1",
            100,
            10,
            0.0,
        );
        assert_eq!(
            s.ingest_marking_dirty(std::slice::from_ref(&first))
                .unwrap()
                .len(),
            1
        );

        // Re-collect: same uuid, model now resolved (parser re-emitted it).
        let mut corrected = first.clone();
        corrected.model = "gpt-5.6-sol".into();
        corrected.pricing_model = "gpt-5.6-sol".into();
        corrected.cost = CostBreakdown {
            input_usd: rust_decimal::Decimal::try_from(0.0001).unwrap(),
            total_usd: rust_decimal::Decimal::try_from(0.0001).unwrap(),
            ..corrected.cost
        };
        let landed = s.ingest_marking_dirty(&[corrected]).unwrap();
        assert_eq!(
            landed.len(),
            1,
            "the corrected re-emission lands as an update"
        );
        let out = s.usage_for_day_device("2026-08-07", "dev1").unwrap();
        assert_eq!(out.len(), 1, "still one row — updated, not duplicated");
        assert_eq!(out[0].model, "gpt-5.6-sol");
        assert_eq!(out[0].pricing_model, "gpt-5.6-sol");
        assert_eq!(
            out[0].cost.total_usd,
            rust_decimal::Decimal::try_from(0.0001).unwrap()
        );
        // The corrected day is dirty: the local artifact (usage-<day>.jsonl)
        // holds the stale "unknown" row and push must recompute it.
        assert!(s.dirty_days().unwrap().contains(&"2026-08-07".to_string()));

        // A known-model row re-ingested with the same uuid stays untouched.
        let again = s.usage_for_day_device("2026-08-07", "dev1").unwrap()[0].clone();
        let mut reemit = again.clone();
        reemit.cost = CostBreakdown {
            input_usd: rust_decimal::Decimal::try_from(9.99).unwrap(),
            total_usd: rust_decimal::Decimal::try_from(9.99).unwrap(),
            ..again.cost
        };
        assert_eq!(
            s.ingest_marking_dirty(&[reemit]).unwrap().len(),
            0,
            "known-model rows are not rewritten"
        );
        let out = s.usage_for_day_device("2026-08-07", "dev1").unwrap();
        assert_eq!(
            out[0].cost.total_usd,
            rust_decimal::Decimal::try_from(0.0001).unwrap(),
            "cost untouched"
        );
    }

    /// The unknown-model self-heal protocol's STORE HALF, pinned on the
    /// corrections entry (the parser half is `CollectResult::corrections`):
    /// a correction — same uuid as a row an earlier pass wrote, model now
    /// resolved — rewrites model + pricing_model + the cost columns and flags
    /// the day dirty. The re-offer on the NEXT pass (the parser re-offers
    /// corrections every pass, since it cannot tell which pre-model rows an
    /// earlier pass wrote — legacy pre-fix rows included) is a NO-OP once the
    /// row carries the model, and a correction for an already-known row never
    /// touches it.
    #[test]
    fn ingest_corrections_marking_dirty_rewrites_only_unknown_rows() {
        let s = mem();
        let day = "2026-08-07";
        // Pass-1 rows: one written "unknown" (healable), one already known.
        let unknown = rec("codex:thread-v1:t1:1", day, "unknown", "dev1", 100, 10, 0.0);
        let known = rec(
            "codex:thread-v1:t1:2",
            day,
            "gpt-5.6-sol",
            "dev1",
            200,
            20,
            0.0,
        );
        s.ingest_marking_dirty(&[unknown.clone(), known.clone()])
            .unwrap();

        // Pass 2: the parser re-offers both rows as corrections (it cannot
        // distinguish them); the guard rewrites ONLY the still-"unknown" one.
        let mut corrected = unknown.clone();
        corrected.model = "gpt-5.6-sol".into();
        corrected.pricing_model = "gpt-5.6-sol".into();
        corrected.cost = CostBreakdown {
            input_usd: rust_decimal::Decimal::try_from(0.0001).unwrap(),
            total_usd: rust_decimal::Decimal::try_from(0.0001).unwrap(),
            ..corrected.cost
        };
        let mut reemit_known = known.clone();
        reemit_known.cost = CostBreakdown {
            input_usd: rust_decimal::Decimal::try_from(9.99).unwrap(),
            total_usd: rust_decimal::Decimal::try_from(9.99).unwrap(),
            ..known.cost
        };
        let landed = s
            .ingest_corrections_marking_dirty(&[corrected.clone(), reemit_known])
            .unwrap();
        assert_eq!(landed.len(), 1, "only the still-'unknown' row is rewritten");
        let rows = s.usage_for_day_device(day, "dev1").unwrap();
        let healed = rows.iter().find(|r| r.uuid == unknown.uuid).unwrap();
        assert_eq!(healed.model, "gpt-5.6-sol");
        assert_eq!(healed.pricing_model, "gpt-5.6-sol");
        assert_eq!(
            healed.cost.total_usd,
            rust_decimal::Decimal::try_from(0.0001).unwrap()
        );
        let known_row = rows.iter().find(|r| r.uuid == known.uuid).unwrap();
        assert_eq!(
            known_row.cost.total_usd,
            rust_decimal::Decimal::ZERO,
            "an already-known row is never touched by a correction"
        );
        // The corrected day is dirty: the local artifact holds the stale row.
        assert!(s.dirty_days().unwrap().contains(&day.to_string()));

        // Pass 3: the parser re-offers the SAME correction again — the row now
        // carries the model, so the guarded upsert is a no-op (this is how
        // every-pass re-offers and legacy pre-fix rows resolve idempotently).
        let again = s.ingest_corrections_marking_dirty(&[corrected]).unwrap();
        assert_eq!(again.len(), 0, "re-offered correction is a no-op");
        let rows = s.usage_for_day_device(day, "dev1").unwrap();
        assert_eq!(rows.len(), 2, "no duplication from re-offers");
    }

    /// The PULL path must keep DO NOTHING semantics: re-ingesting a row never
    /// modifies it, even when the pulled copy carries a resolved model — the
    /// guarded upsert is a local-collect correction, not an import overwrite.
    #[test]
    fn ingest_pull_path_never_touches_existing_rows() {
        let s = mem();
        let first = rec(
            "codex:thread-v1:t1:1",
            "2026-08-07",
            "unknown",
            "dev1",
            100,
            10,
            0.0,
        );
        s.ingest(std::slice::from_ref(&first)).unwrap();
        let mut corrected = first.clone();
        corrected.model = "gpt-5.6-sol".into();
        corrected.pricing_model = "gpt-5.6-sol".into();
        assert_eq!(
            s.ingest(&[corrected]).unwrap().len(),
            0,
            "pull re-ingest still dedupes"
        );
        let out = s.usage_for_day_device("2026-08-07", "dev1").unwrap();
        assert_eq!(
            out[0].model, "unknown",
            "pull never modifies an existing row"
        );
    }

    /// Regression: the same uuid on two DIFFERENT devices must both be kept —
    /// dedup is scoped by the (uuid, device_id) primary key, not uuid alone. An
    /// old uuid-only PK dropped the peer device's row, so a source event replayed
    /// under two device ids (one ~/.claude/projects scanned twice, a restored
    /// opencode.db) silently erased one device. Both devices must be visible
    /// afterwards.
    #[test]
    fn ingest_keeps_same_uuid_across_devices() {
        let s = mem();
        let uuid_x = "codex:thread-v1:sess-1:1";
        let a = rec(
            uuid_x,
            "2026-07-30",
            "gpt-5.2-codex",
            "aaaaaa000001",
            100,
            10,
            0.0,
        );
        let b = rec(
            uuid_x,
            "2026-07-30",
            "gpt-5.2-codex",
            "bbbbbb000002",
            200,
            20,
            0.0,
        );
        assert_eq!(s.ingest(std::slice::from_ref(&a)).unwrap().len(), 1);
        // Same uuid, different device ⇒ must still ingest (previously dropped).
        assert_eq!(s.ingest(std::slice::from_ref(&b)).unwrap().len(), 1);

        let logs = s
            .query_logs(&LogsQuery {
                filter: UsageFilter::default(),
                limit: 10,
                offset: 0,
            })
            .unwrap();
        let devices: Vec<String> = logs.iter().map(|l| l.device_id.clone()).collect();
        assert!(devices.contains(&"aaaaaa000001".to_string()));
        assert!(devices.contains(&"bbbbbb000002".to_string()));

        // Re-ingesting the SAME (uuid, device) is still idempotent (re-pull dedup).
        assert_eq!(s.ingest(std::slice::from_ref(&a)).unwrap().len(), 0);
    }

    /// The three hand-written column positions (the INSERT, the per-day SELECT,
    /// and `row_to_usage_record`'s positional reads) must stay aligned with the
    /// schema constant — a column added there but missed in one spot silently
    /// misaligns the positional reads (single source of truth). This round-trips
    /// a full sentinel row through the PRODUCTION paths (`ingest_marking_dirty`
    /// → `usage_for_day_device`), so every field is compared non-trivially:
    /// any drift (missing column, swapped order, off-by-one index) breaks the
    /// equality instead of being papered over by defaults.
    #[test]
    fn usage_row_roundtrips_through_production_paths() {
        let s = mem();
        let r = UsageRecord {
            uuid: "sentinel-uuid-001".into(),
            timestamp: "2026-07-13T12:34:56Z".into(),
            day: "2026-07-13".into(),
            model: "model-sentinel".into(),
            pricing_model: "pricing-sentinel".into(),
            source: "source-sentinel".into(),
            session_id: "session-sentinel".into(),
            device_id: "dev-sentinel".into(),
            tokens: TokenCounts {
                input: 123,
                output: 456,
                cache_creation: 78,
                cache_read: 90,
            },
            server_tool_use: ServerToolUse {
                web_search: 7,
                web_fetch: 8,
            },
            stop_reason: "stop-sentinel".into(),
            service_tier: "tier-sentinel".into(),
            iterations: 42,
            cost: CostBreakdown {
                input_usd: "1.11".parse().unwrap(),
                output_usd: "2.22".parse().unwrap(),
                cache_read_usd: "3.33".parse().unwrap(),
                cache_creation_usd: "4.44".parse().unwrap(),
                total_usd: "11.10".parse().unwrap(),
            },
        };
        s.ingest_marking_dirty(std::slice::from_ref(&r)).unwrap();
        let out = s
            .usage_for_day_device("2026-07-13", "dev-sentinel")
            .unwrap();
        assert_eq!(out.len(), 1, "sentinel row landed");
        assert_eq!(
            out[0], r,
            "every usage_records column round-trips through the production paths"
        );
    }

    #[test]
    fn turn_durations_ingest_and_aggregate() {
        let s = mem();
        s.ingest_turn_durations(&[
            TurnDuration {
                uuid: "t1".into(),
                timestamp: "2026-07-13T10:00:00Z".into(),
                day: "2026-07-13".into(),
                session_id: "s1".into(),
                device_id: "d".into(),
                duration_ms: 100_000,
            },
            TurnDuration {
                uuid: "t2".into(),
                timestamp: "2026-07-13T11:00:00Z".into(),
                day: "2026-07-13".into(),
                session_id: "s1".into(),
                device_id: "d".into(),
                duration_ms: 200_000,
            },
        ])
        .unwrap();
        // Same uuid dedupes (INSERT OR IGNORE).
        s.ingest_turn_durations(&[TurnDuration {
            uuid: "t1".into(),
            timestamp: "2026-07-13T10:00:00Z".into(),
            day: "2026-07-13".into(),
            session_id: "s1".into(),
            device_id: "d".into(),
            duration_ms: 999_999,
        }])
        .unwrap();
        let stats = s.query_stats(&UsageFilter::default()).unwrap();
        assert_eq!(stats.turn_count, 2);
        assert!((stats.avg_turn_duration_ms - 150_000.0).abs() < 1e-9);
    }

    // ---- incremental scan cursors ----

    #[test]
    fn scan_progress_save_load_roundtrip() {
        let s = mem();
        let mut delta = ScanProgressDelta::new();
        delta.insert(
            "C:/a.jsonl".into(),
            FileCursor {
                last_modified: 1_000,
                last_line_offset: 5,
            },
        );
        delta.insert(
            "C:/b.jsonl".into(),
            FileCursor {
                last_modified: 2_000,
                last_line_offset: 10,
            },
        );
        s.save_scan_progress(&delta).unwrap();
        let loaded = s.load_scan_progress().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("C:/a.jsonl").unwrap().last_line_offset, 5);
        assert_eq!(loaded.get("C:/b.jsonl").unwrap().last_modified, 2_000);
    }

    #[test]
    fn scan_progress_upsert_overwrites_on_conflict() {
        let s = mem();
        let mut delta = ScanProgressDelta::new();
        delta.insert(
            "C:/a.jsonl".into(),
            FileCursor {
                last_modified: 1,
                last_line_offset: 5,
            },
        );
        s.save_scan_progress(&delta).unwrap();
        // Same path, advanced cursor — UPSERT must overwrite, not duplicate.
        delta.insert(
            "C:/a.jsonl".into(),
            FileCursor {
                last_modified: 2,
                last_line_offset: 10,
            },
        );
        s.save_scan_progress(&delta).unwrap();
        let loaded = s.load_scan_progress().unwrap();
        assert_eq!(loaded.len(), 1, "upsert overwrites, not inserts");
        let c = loaded.get("C:/a.jsonl").unwrap();
        assert_eq!(c.last_modified, 2);
        assert_eq!(c.last_line_offset, 10);
    }

    #[test]
    fn scan_progress_load_empty_on_fresh_db() {
        let s = mem();
        assert!(
            s.load_scan_progress().unwrap().is_empty(),
            "fresh DB has no cursors ⇒ first collect is a full scan"
        );
    }

    #[test]
    fn scan_progress_save_empty_delta_is_noop() {
        let s = mem();
        let delta = ScanProgressDelta::new();
        s.save_scan_progress(&delta).unwrap();
        assert!(s.load_scan_progress().unwrap().is_empty());
    }
}
