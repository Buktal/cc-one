//! Ingest pipeline: RawUsage → UsageRecord (cost computed) and RawTurnDuration
//! → TurnDuration, written to the SQLite Local Store, with the rows' days
//! flagged dirty for the push path.
//!
//! The parser emits raw per-call events + raw per-turn durations (no cost, no
//! device). Here we attach the owning device_id, derive the day bucket and
//! pricing_model, compute cost via the pure CostCalculator, and write the new
//! rows to SQLite (deduped by the `(uuid, device_id)` primary key). The JSONL
//! Artifact and session snapshot are derived projections owned by the
//! `collect::artifact` and `sessions::session_snapshot` modules — collect never
//! touches them; push recomputes them from the store.

use crate::config::Paths;
use crate::db::Store;
use crate::error::AppResult;
use crate::model::{RawSession, SessionMessage, TurnDuration, UsageRecord};
use crate::pricing::{CostCalculator, PricingBook};
use crate::sessions::snapshot_policy::{decide_snapshot_action, SnapshotAction};
use crate::source_parser::{CollectResult, RawTurnDuration, RawUsage};

/// Summary of one ingest run.
#[derive(Debug, Clone, Default, serde::Serialize, specta::Type)]
pub struct IngestReport {
    pub source: String,
    pub events_collected: u32,
    pub rows_inserted: u32,
    pub turn_durations_collected: u32,
    pub turn_durations_inserted: u32,
    pub files_scanned: u32,
    pub lines_skipped: u32,
}

/// Turn a raw per-call event into a full stored record (cost + device + day).
/// Pure: given the same book, deterministic.
pub(crate) fn recordify(raw: &RawUsage, device_id: &str, book: &PricingBook) -> UsageRecord {
    let pricing_model = crate::model::normalize_model_key(&raw.model);
    let rate = book.resolve(&raw.model);
    let cost = CostCalculator::calc(raw.tokens, rate);
    UsageRecord {
        uuid: raw.uuid.clone(),
        day: UsageRecord::day_from_timestamp(&raw.timestamp),
        timestamp: raw.timestamp.clone(),
        model: raw.model.clone(),
        pricing_model,
        source: raw.source.clone(),
        session_id: raw.session_id.clone(),
        device_id: device_id.to_string(),
        tokens: raw.tokens,
        server_tool_use: raw.server_tool_use,
        stop_reason: raw.stop_reason.clone(),
        service_tier: raw.service_tier.clone(),
        iterations: raw.iterations,
        cost,
    }
}

/// Turn a raw per-turn duration into a stored TurnDuration (attach device + day).
fn turn_durationify(raw: &RawTurnDuration, device_id: &str) -> TurnDuration {
    TurnDuration {
        uuid: raw.uuid.clone(),
        day: UsageRecord::day_from_timestamp(&raw.timestamp),
        timestamp: raw.timestamp.clone(),
        device_id: device_id.to_string(),
        duration_ms: raw.duration_ms,
    }
}

/// Ingest a parser's collect result: compute cost + day, then write the rows
/// to the SQLite Local Store, flagging each new row's day dirty in the same
/// transaction. The usage/turn JSONL Artifact and session snapshot are NOT
/// touched here — they are derived snapshots the push path recomputes from the
/// store per dirty day / favorited session. SQLite is the single source of
/// truth; the scan cursor advances only after the ingest commits, so a failed
/// ingest re-parses the same source lines next collect (store dedup). `paths` is
/// NOT for transcript writes — collect is store-only; it is used only by the
/// ghost-session reconcile (best-effort unlink of a vanished session's snapshot
/// file, which the push path owns).
/// Returns a summary.
pub fn ingest_collected(
    store: &Store,
    paths: &Paths,
    device_id: &str,
    book: &PricingBook,
    result: CollectResult,
) -> AppResult<IngestReport> {
    // Corrections are re-emitted events (rows an earlier pass already wrote) —
    // counted as collected like before the channel split, so the report does
    // not shrink.
    let events_collected = (result.events.len() + result.corrections.len()) as u32;
    let turn_durations_collected = result.turn_durations.len() as u32;
    let source = result.source.clone();

    // Per-call usage records → store (+ mark their days dirty, same tx).
    let records: Vec<UsageRecord> = result
        .events
        .iter()
        .map(|r| recordify(r, device_id, book))
        .collect();
    let inserted = store.ingest_marking_dirty(&records)?;

    // Correction candidates (Codex unknown-model self-heal) → the guarded
    // upsert that rewrites exactly the rows still reading model='unknown'
    // (the protocol's store half — see
    // `Store::ingest_corrections_marking_dirty`). Rewritten rows count as
    // inserted: they ARE real store changes, and their days are flagged dirty
    // so the push path recomputes the derived artifact.
    let correction_records: Vec<UsageRecord> = result
        .corrections
        .iter()
        .map(|r| recordify(r, device_id, book))
        .collect();
    let mut landed = store.ingest_corrections_marking_dirty(&correction_records)?;
    landed.extend(inserted);
    let inserted = landed;

    // Per-turn durations (separate grain) → store (+ mark dirty, same tx).
    let turns: Vec<TurnDuration> = result
        .turn_durations
        .iter()
        .map(|t| turn_durationify(t, device_id))
        .collect();
    let turns_inserted = store.ingest_turn_durations_marking_dirty(&turns)?;

    // Sessions (Claude only in this phase; empty for other sources). Refreshes
    // the sessions table (system data; user data preserved by UPSERT) and writes
    // all transcript messages to the store (favorited or not); the per-session
    // snapshot is the push path's derived concern, not collect's.
    ingest_sessions(store, device_id, &result.sessions, &result.messages)?;

    // File-backed reality check: drop session rows (and their transcript
    // files) whose source file no longer exists. Runs only when the ingest
    // above succeeded — a failed ingest propagates via `?` and never
    // reconciles (no partial state).
    if !result.session_ids.is_empty() {
        reconcile_session_data(store, paths, device_id, &result.source, &result.session_ids)?;
    }

    Ok(IngestReport {
        source,
        events_collected,
        rows_inserted: inserted.len() as u32,
        turn_durations_collected,
        turn_durations_inserted: turns_inserted.len() as u32,
        files_scanned: result.files_scanned,
        lines_skipped: result.lines_skipped,
    })
}

/// Reconcile THIS device's `source` sessions against the files actually seen:
/// delete rows whose id is not in `seen_ids`, then best-effort remove their
/// snapshot files (`sessions/<id>.jsonl`). The session row and its transcript
/// are one unit — a ghost row's transcript would otherwise linger forever.
/// Returns the number of sessions removed. Scoped by `(device_id, source)` in
/// SQL, so a peer's rows and other sources are never touched. `seen_ids` comes
/// from the parser's DISCOVERED files (not the parsed output — the mtime gate
/// skips unchanged files, so the parsed set would shrink to zero on a no-change
/// collect and wipe real sessions).
fn reconcile_session_data(
    store: &Store,
    paths: &Paths,
    device_id: &str,
    source: &str,
    seen_ids: &[String],
) -> AppResult<usize> {
    let ghosts = store.reconcile_sessions(device_id, source, seen_ids)?;
    // A ghost session's row is gone ⇒ it cannot be favorited ⇒ the snapshot
    // policy's action is `Remove`. Route through `decide_snapshot_action`
    // rather than a bare `remove_file` so this — the third enforcement site,
    // alongside push's `decide_snapshot_action` and pull's
    // `presence_mismatches` — shares the one definition of "snapshot file
    // exists ⇔ favorited"; a future change to what `Remove` means then can't
    // silently skip collect. Matching (not an `if`) forces this site to
    // reconsider whenever a new action is added.
    match decide_snapshot_action(false) {
        SnapshotAction::Remove => {
            for id in &ghosts {
                // Best-effort, on purpose: an unlink failure (permissions, etc.)
                // must not fail the collect — the row is already gone; the orphan
                // file is retried next pass. Push handles the same `Remove` by
                // failing loud (`?`) because a push can retry; the two error
                // semantics differ by design.
                let _ = std::fs::remove_file(paths.session_snapshot_path(device_id, id));
            }
        }
        // A ghost row is never favorited, so Write is unreachable today.
        SnapshotAction::Write => {}
    }
    if !ghosts.is_empty() {
        eprintln!(
            "[cc-one] reconciled {device_id}/{source}: removed {} ghost session(s)",
            ghosts.len()
        );
    }
    Ok(ghosts.len())
}

// ---------------- Sessions (local session data + transcript) ----------------
//
// Sessions are LOCAL data: the `sessions` SQLite table (system data refreshed
// by re-extract, user data preserved by UPSERT) + ALL transcript messages in
// `session_messages` (db single source of truth — favorited or not). Favorited
// sessions' derived `sessions/<id>.jsonl` snapshots are a push-path concern
// (see `session_snapshot`); collect never writes them.

/// Ingest a parser's session output:
///   1. Refresh system data in the `sessions` table (UPSERT preserves user data).
///   2. Write ALL transcript messages to `session_messages` (db single source of
///      truth — favorited or not) and mark their sessions dirty for the push path.
///
/// The jsonl snapshot is NO LONGER written here — the push path recomputes it
/// from the store (`session_snapshot::recompute_session_snapshot`), so collect is
/// store-only. The favorites gate lives entirely in the push path: a
/// non-favorited session's messages still land in the db (readable locally), but
/// no `sessions/<id>.jsonl` is produced for it.
pub(crate) fn ingest_sessions(
    store: &Store,
    device_id: &str,
    sessions: &[RawSession],
    messages: &[SessionMessage],
) -> AppResult<()> {
    if sessions.is_empty() && messages.is_empty() {
        return Ok(());
    }

    // SQLite: refresh system data only (UPSERT preserves user data) — the
    // "re-extract never overwrites user data" invariant, encoded in SQL.
    for s in sessions {
        store.upsert_session(device_id, s)?;
    }

    // All transcript messages → db. EVERY session lands here (favorited or not):
    // SQLite is the single source of truth for 原文. Sessions with new rows are
    // flagged dirty in the same transaction so the push path recomputes their
    // snapshots; the favorites gate (write vs delete the jsonl) is the push
    // path's concern, not collect's.
    if !messages.is_empty() {
        store.ingest_session_messages_marking_dirty(device_id, messages)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ServerToolUse, TokenCounts};
    use crate::pricing::seed_book;
    use crate::source_parser::RawTurnDuration;

    fn raw(uuid: &str, model: &str) -> RawUsage {
        RawUsage {
            uuid: uuid.into(),
            timestamp: "2026-07-13T16:55:22.467Z".into(),
            model: model.into(),
            source: "claude_code".into(),
            session_id: String::new(),
            tokens: TokenCounts {
                input: 1000,
                output: 500,
                cache_creation: 0,
                cache_read: 0,
            },
            server_tool_use: ServerToolUse::default(),
            stop_reason: "end_turn".into(),
            service_tier: "standard".into(),
            iterations: 0,
        }
    }

    fn raw_turn(uuid: &str) -> RawTurnDuration {
        RawTurnDuration {
            uuid: uuid.into(),
            timestamp: "2026-07-13T16:55:00Z".into(),
            duration_ms: 123_456,
        }
    }

    #[test]
    fn recordify_attaches_day_pricing_model_and_cost() {
        let book = seed_book();
        let r = recordify(&raw("u1", "glm-5.2[1m]"), "0123456789ab", &book);
        assert_eq!(r.uuid, "u1");
        assert_eq!(r.device_id, "0123456789ab");
        assert_eq!(r.day, "2026-07-13");
        assert_eq!(
            r.pricing_model, "glm-5.2",
            "bracket stripped for pricing lookup"
        );
        assert_eq!(r.model, "glm-5.2[1m]", "original billed model preserved");
        // New per-call fields pass through.
        assert_eq!(r.stop_reason, "end_turn");
        assert_eq!(r.service_tier, "standard");
        // glm-5.2: input 0.60/1M × 1000 + output 2.20/1M × 500 = 0.0006 + 0.0011.
        assert!(
            (r.cost.total_f64() - 0.0017).abs() < 1e-9,
            "cost = {}",
            r.cost.total_f64()
        );
    }

    #[test]
    fn recordify_is_zero_cost_for_unknown_model() {
        let book = seed_book();
        let r = recordify(&raw("u2", "no-such-model"), "0123456789ab", &book);
        assert_eq!(r.cost.total_f64(), 0.0);
    }

    /// The collect path flags the days of newly ingested rows dirty (in the same
    /// tx as the write) AND leaves the Artifact unwritten — the store is the
    /// single source of truth now; the push path materializes files. Proves both:
    /// days flagged, and no file appears from collect.
    #[test]
    fn ingest_collected_flags_dirty_days_and_writes_no_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = seed_book();
        let dev = "0123456789ab";
        let d1 = raw("u1", "glm-5.2");
        let d2 = RawUsage {
            timestamp: "2026-07-14T16:55:22.467Z".into(),
            ..raw("u2", "glm-5.2")
        };
        let result = CollectResult {
            source: "claude_code".into(),
            events: vec![d1, d2],
            corrections: vec![],
            turn_durations: vec![raw_turn("td1")],
            files_scanned: 1,
            lines_skipped: 0,
            sessions: vec![],
            messages: vec![],
            session_ids: vec![],
        };
        ingest_collected(&store, &paths, dev, &book, result).unwrap();
        assert_eq!(
            store.dirty_days().unwrap(),
            vec!["2026-07-13".to_string(), "2026-07-14".to_string()],
            "D1 (usage + turn) and D2 flagged, deduped + sorted"
        );
        // collect writes the store, NOT the Artifact — no file exists yet.
        assert!(
            !paths
                .device_data_dir(dev)
                .join("usage-2026-07-13.jsonl")
                .exists(),
            "collect must not write the Artifact (push recomputes it)"
        );
    }

    #[test]
    fn ingest_collected_dedups_via_store_pk() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = seed_book();
        let result = CollectResult {
            source: "claude_code".into(),
            events: vec![raw("dup", "glm-5.2")],
            corrections: vec![],
            turn_durations: vec![raw_turn("td1")],
            files_scanned: 1,
            lines_skipped: 0,
            sessions: vec![],
            messages: vec![],
            session_ids: vec![],
        };
        let rep1 = ingest_collected(&store, &paths, "0123456789ab", &book, result.clone()).unwrap();
        assert_eq!(rep1.rows_inserted, 1);
        assert_eq!(rep1.events_collected, 1);
        assert_eq!(rep1.turn_durations_collected, 1);
        assert_eq!(rep1.turn_durations_inserted, 1);
        // Same uuids again ⇒ fully deduped.
        let rep2 = ingest_collected(&store, &paths, "0123456789ab", &book, result).unwrap();
        assert_eq!(rep2.rows_inserted, 0);
        assert_eq!(rep2.turn_durations_inserted, 0);
    }

    // ---- session helpers + reconcile tests ----

    fn sys_session(id: &str, last_active_at: &str) -> RawSession {
        RawSession {
            id: id.into(),
            source: "claude_code".into(),
            project_dir: "/proj".into(),
            title_orig: "orig-title".into(),
            started_at: "2026-08-01T00:00:00.000Z".into(),
            last_active_at: last_active_at.into(),
            agent_type: String::new(),
        }
    }

    fn msg(uuid: &str, session_id: &str, content: &str) -> SessionMessage {
        SessionMessage {
            uuid: uuid.into(),
            session_id: session_id.into(),
            role: crate::model::SessionMessageRole::User,
            ts: "2026-08-01T00:00:00.000Z".into(),
            model: None,
            name: None,
            content: content.into(),
        }
    }

    /// Reconcile deletes ghost session rows AND their transcript files; a
    /// real (seen) favorited session keeps both.
    #[test]
    fn reconcile_removes_ghost_rows_and_their_transcripts() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let dev = "0123456789ab";

        // Two favorited sessions; their messages land in the db (collect is
        // store-only — the jsonl is a derived snapshot the push path writes).
        ingest_sessions(
            &store,
            dev,
            &[
                sys_session("real", "2026-08-01T01:00:00.000Z"),
                sys_session("ghost", "2026-08-01T01:00:00.000Z"),
            ],
            &[],
        )
        .unwrap();
        store.set_session_favorited(dev, "real", true).unwrap();
        store.set_session_favorited(dev, "ghost", true).unwrap();
        ingest_sessions(
            &store,
            dev,
            &[
                sys_session("real", "2026-08-01T01:00:00.000Z"),
                sys_session("ghost", "2026-08-01T01:00:00.000Z"),
            ],
            &[msg("m1", "real", "hi"), msg("m2", "ghost", "bye")],
        )
        .unwrap();
        // Push materializes both favorited sessions' jsonl snapshots.
        crate::sessions::session_snapshot::recompute_session_snapshot(&store, &paths, dev, "real")
            .unwrap();
        crate::sessions::session_snapshot::recompute_session_snapshot(&store, &paths, dev, "ghost")
            .unwrap();
        assert!(
            paths.session_snapshot_path(dev, "real").exists()
                && paths.session_snapshot_path(dev, "ghost").exists(),
            "both snapshots written (both favorited)"
        );

        // Next collect sees only `real` → `ghost` row + jsonl + messages vanish.
        let removed =
            reconcile_session_data(&store, &paths, dev, "claude_code", &["real".to_string()])
                .unwrap();
        assert_eq!(removed, 1);
        let ids: Vec<String> = store
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["real"], "ghost row deleted");
        assert!(
            !paths.session_snapshot_path(dev, "ghost").exists(),
            "ghost jsonl removed"
        );
        assert!(
            paths.session_snapshot_path(dev, "real").exists(),
            "real jsonl untouched"
        );

        // The db tracks the same unit: the ghost's messages leave session_messages
        // too, not just the jsonl artifact on disk.
        assert!(
            store
                .query_session_messages(dev, "ghost")
                .unwrap()
                .is_empty(),
            "ghost session_messages removed with its row"
        );
        assert_eq!(
            store.query_session_messages(dev, "real").unwrap().len(),
            1,
            "real session_messages untouched"
        );
    }

    /// Full-collect flow: session s2 was on disk at the first collect, deleted
    /// (or superseded) before the second — its row and transcript disappear
    /// while s1 (still seen) survives.
    #[test]
    fn ingest_collected_reconciles_across_two_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = seed_book();
        let dev = "0123456789ab";

        // Pass 1: both sessions seen (rows created, no messages yet — the
        // favorited flag must be set AFTER the row exists for the transcript
        // to land).
        let pass1 = CollectResult {
            source: "claude_code".into(),
            events: vec![],
            corrections: vec![],
            turn_durations: vec![],
            sessions: vec![
                sys_session("s1", "2026-08-01T01:00:00.000Z"),
                sys_session("s2", "2026-08-01T01:00:00.000Z"),
            ],
            messages: vec![],
            files_scanned: 2,
            lines_skipped: 0,
            session_ids: vec!["s1".into(), "s2".into()],
        };
        ingest_collected(&store, &paths, dev, &book, pass1).unwrap();
        store.set_session_favorited(dev, "s2", true).unwrap();
        // Pass 1b: messages arrive for the (now favorited) s2 → transcript.
        let pass1b = CollectResult {
            source: "claude_code".into(),
            events: vec![],
            corrections: vec![],
            turn_durations: vec![],
            sessions: vec![
                sys_session("s1", "2026-08-01T01:00:00.000Z"),
                sys_session("s2", "2026-08-01T01:00:00.000Z"),
            ],
            messages: vec![msg("m1", "s1", "a"), msg("m2", "s2", "b")],
            files_scanned: 2,
            lines_skipped: 0,
            session_ids: vec!["s1".into(), "s2".into()],
        };
        ingest_collected(&store, &paths, dev, &book, pass1b).unwrap();
        // Push writes s2's derived jsonl (collect no longer touches it).
        crate::sessions::session_snapshot::recompute_session_snapshot(&store, &paths, dev, "s2")
            .unwrap();
        assert!(paths.session_snapshot_path(dev, "s2").exists());

        // Pass 2: s2's file is gone from disk; only s1 is seen. Its row +
        // transcript must be reconciled away even though s2 was favorited.
        let pass2 = CollectResult {
            source: "claude_code".into(),
            events: vec![],
            corrections: vec![],
            turn_durations: vec![],
            sessions: vec![sys_session("s1", "2026-08-02T01:00:00.000Z")],
            messages: vec![],
            files_scanned: 1,
            lines_skipped: 0,
            session_ids: vec!["s1".into()],
        };
        ingest_collected(&store, &paths, dev, &book, pass2).unwrap();

        let ids: Vec<String> = store
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["s1"], "s2 reconciled away after its file vanished");
        assert!(
            !paths.session_snapshot_path(dev, "s2").exists(),
            "s2 transcript removed with its row"
        );
    }

    /// All transcript messages land in the db (`session_messages`), favorited or
    /// not — SQLite is the single source of truth for 原文. The favorites gate
    /// applies only to the derived jsonl snapshot (a push-path concern); the db
    /// holds every session so a non-favorited session can still be read.
    #[test]
    fn ingest_sessions_writes_all_messages_to_db_regardless_of_favorite() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let dev = "0123456789ab";

        let fav = sys_session("fav", "2026-08-01T01:00:00.000Z");
        let plain = sys_session("plain", "2026-08-01T01:00:00.000Z");
        ingest_sessions(
            &store,
            dev,
            &[fav, plain],
            &[msg("m1", "fav", "hello"), msg("m2", "plain", "world")],
        )
        .unwrap();

        // Neither session is favorited, yet BOTH land in the db.
        assert_eq!(
            store.query_session_messages(dev, "fav").unwrap().len(),
            1,
            "favorited session's messages in db"
        );
        assert_eq!(
            store.query_session_messages(dev, "plain").unwrap().len(),
            1,
            "non-favorited session's messages ALSO in db (原文 for all sessions)"
        );
        // Both flagged dirty so the push path recomputes their snapshots.
        let dirty = store.dirty_sessions().unwrap();
        assert!(dirty.contains(&"fav".to_string()));
        assert!(dirty.contains(&"plain".to_string()));
    }
}
