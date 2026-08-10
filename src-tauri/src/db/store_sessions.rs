//! Sessions table: per-session UPSERT + per-field user-data setters.
//!
//! Hosts the shared `upsert_session_row` / `SessionUpsertPolicy` core used by
//! both collect ([`Store::upsert_session`]) and pull
//! ([`super::transcript::Store::import_session_snapshot`]).

use super::store_transcript::mark_sessions_dirty;
use super::*;

impl super::Store {
    // ---------------- Sessions ----------------

    /// Refresh a session's SYSTEM-data columns only. On conflict (same id +
    /// device_id), the [`SessionUpsertPolicy::RefreshSystemOnly`] clause
    /// updates exactly the refreshable columns (source / project_dir /
    /// title_orig / started_at / last_active_at) — it MUST NOT touch
    /// `custom_title` / `favorited` / `synced_group_id` / `local_group_id`.
    /// This is the SQLite-side encoding of the "re-extract never overwrites
    /// user data" invariant; a regression test in this module pins it.
    pub fn upsert_session(&self, device_id: &str, system: &SessionSystemData) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        upsert_session_row(
            &tx,
            device_id,
            system,
            false, // favorited — a freshly collected session is not favorited
            "",    // synced_group_id — collect never assigns a synced group
            SessionUpsertPolicy::RefreshSystemOnly,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Read a session's favorited flag. `None` when the session is not yet in
    /// the table — the caller treats that as not-favorited. Read on the push
    /// path to decide whether to write this session's git snapshot —
    /// `snapshot_policy::decide_snapshot_action` writes it when favorited and
    /// removes it when not (the "snapshot exists ⇔ favorited" invariant).
    pub fn get_session_favorited(
        &self,
        device_id: &str,
        session_id: &str,
    ) -> AppResult<Option<bool>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let fav = conn
            .query_row(
                "SELECT favorited FROM sessions WHERE id = ?1 AND device_id = ?2",
                params![session_id, device_id],
                |r| r.get::<_, i64>(0).map(|v| v != 0),
            )
            .optional()?;
        Ok(fav)
    }

    /// Set a session's favorited flag (user action). The push path materializes
    /// the session's jsonl snapshot when this is true and REMOVES it when false,
    /// so the toggle must flag the session dirty in the SAME transaction —
    /// otherwise a favorite (or un-favorite) would never reach git (the exact
    /// "miss git" failure the same-tx marking guards against for usage rows).
    pub fn set_session_favorited(
        &self,
        device_id: &str,
        session_id: &str,
        favorited: bool,
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET favorited = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, favorited as i64],
        )?;
        let mut dirty = std::collections::BTreeSet::new();
        dirty.insert(session_id.to_string());
        mark_sessions_dirty(&tx, &dirty)?;
        tx.commit()?;
        Ok(())
    }

    /// Set/clear a session's custom title. `None` or empty clears it (reverts to
    /// `title_orig` for display).
    pub fn set_session_custom_title(
        &self,
        device_id: &str,
        session_id: &str,
        title: Option<&str>,
    ) -> AppResult<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let t = title.unwrap_or("").trim();
        conn.execute(
            "UPDATE sessions SET custom_title = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, t],
        )?;
        Ok(())
    }

    /// Set/clear a session's local group (device-private).
    pub fn set_session_local_group(
        &self,
        device_id: &str,
        session_id: &str,
        group_id: Option<&str>,
    ) -> AppResult<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let g = group_id.unwrap_or("");
        conn.execute(
            "UPDATE sessions SET local_group_id = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, g],
        )?;
        Ok(())
    }

    /// Set/clear a session's synced group (cross-device — the synced_group_id
    /// rides the jsonl snapshot's meta line, so a change must flag the session
    /// dirty in the same transaction to reach git on the next push).
    pub fn set_session_synced_group(
        &self,
        device_id: &str,
        session_id: &str,
        group_id: Option<&str>,
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let g = group_id.unwrap_or("");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET synced_group_id = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, g],
        )?;
        let mut dirty = std::collections::BTreeSet::new();
        dirty.insert(session_id.to_string());
        mark_sessions_dirty(&tx, &dirty)?;
        tx.commit()?;
        Ok(())
    }

    /// Delete this device's sessions for `source` whose id was NOT seen by the
    /// latest collect — the file-backed reality check that keeps the sessions
    /// table from accumulating ghosts (deleted session files, previously
    /// scanned agent sub-sessions). Returns the deleted ids so the caller can
    /// also remove their transcript files. An empty `seen_ids` is a NO-OP —
    /// a transiently invisible source dir must not wipe real rows (the caller
    /// only passes a non-empty set anyway; this is the second line of defense).
    /// One transaction; `(device_id, source, id)` scoping never touches a
    /// peer's rows or another source.
    pub fn reconcile_sessions(
        &self,
        device_id: &str,
        source: &str,
        seen_ids: &[String],
    ) -> AppResult<Vec<String>> {
        if seen_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        // The seen set rides as a JSON array through json_each — a single
        // parameter with no SQLite variable-count ceiling for large sets.
        let json = serde_json::to_string(seen_ids)
            .map_err(|e| AppError::Internal(format!("reconcile seen ids: {e}")))?;
        let ghosts: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM sessions \
                 WHERE device_id = ?1 AND source = ?2 \
                   AND id NOT IN (SELECT value FROM json_each(?3))",
            )?;
            let rows = stmt.query_map(params![device_id, source, json], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !ghosts.is_empty() {
            tx.execute(
                "DELETE FROM sessions \
                 WHERE device_id = ?1 AND source = ?2 \
                   AND id NOT IN (SELECT value FROM json_each(?3))",
                params![device_id, source, json],
            )?;
            // A ghost session's messages are dead weight too — drop them in the
            // same transaction so the row and its transcript never split apart.
            // `session_messages` has no `source` column, so scope by device plus
            // the ghost id set (the very rows just deleted from `sessions`).
            let ghost_json = serde_json::to_string(&ghosts)
                .map_err(|e| AppError::Internal(format!("reconcile ghost ids: {e}")))?;
            tx.execute(
                "DELETE FROM session_messages \
                 WHERE device_id = ?1 \
                   AND session_id IN (SELECT value FROM json_each(?2))",
                params![device_id, ghost_json],
            )?;
        }
        tx.commit()?;
        Ok(ghosts)
    }
}

// ---- sessions-table UPSERT core (shared by collect + pull) ----
//
// `Store::upsert_session` (collect / re-extract) and
// `Store::import_session_snapshot` (pull) both write one row of the SAME
// `sessions` table with an identical 12-column INSERT. They differ ONLY in
// which columns an existing row gets refreshed on conflict — so that
// difference lives in one typed place (`SessionUpsertPolicy`) instead of two
// `ON CONFLICT` clauses kept in sync only by comments.

/// Which columns an [`upsert_session_row`] conflict-update refreshes. The two
/// sessions-table UPSERT callers differ ONLY here, so this enum is the single
/// typed home of that difference.
pub(super) enum SessionUpsertPolicy {
    /// Collect / re-extract: refresh the 6 system-data columns only. Never
    /// touches user-data columns — the "re-extract must not overwrite user
    /// edits" invariant.
    RefreshSystemOnly,
    /// Pull / import: refresh the 6 system columns AND `favorited` /
    /// `synced_group_id` — a peer's snapshot is authoritative for its own
    /// row's favorites-track fields. `custom_title` / `local_group_id` stay
    /// device-local either way (never carried by a snapshot).
    RefreshSystemAndFavorites,
}

impl SessionUpsertPolicy {
    /// The `ON CONFLICT(id, device_id) DO UPDATE SET` clause this policy
    /// drives. Every policy refreshes the 6 shared system-data columns; pull
    /// additionally takes the two favorites-track columns. The device-local
    /// columns (`custom_title`, `local_group_id`) never appear here.
    fn conflict_set(&self) -> String {
        // The refreshable system-data columns — shared by both policies, so
        // declared once (single source of truth).
        const SYSTEM: &str = "source=excluded.source, project_dir=excluded.project_dir, \
                              title_orig=excluded.title_orig, started_at=excluded.started_at, \
                              last_active_at=excluded.last_active_at, \
                              agent_type=excluded.agent_type";
        match self {
            SessionUpsertPolicy::RefreshSystemOnly => SYSTEM.to_string(),
            SessionUpsertPolicy::RefreshSystemAndFavorites => format!(
                "{SYSTEM}, favorited=excluded.favorited, synced_group_id=excluded.synced_group_id"
            ),
        }
    }
}

/// UPSERT one `sessions` row keyed by `(sys.id, device_id)` — the shared core
/// of [`Store::upsert_session`] (collect) and [`Store::import_session_snapshot`]
/// (pull). The INSERT lists all 12 columns (single source); on conflict,
/// `policy` picks which columns refresh. `custom_title` and `local_group_id`
/// seed as empty on INSERT for every caller (neither is carried: `custom_title`
/// is a local edit, `local_group_id` never enters git). `favorited` /
/// `synced_group_id` seed with the caller's values — defaults for collect, the
/// snapshot's for pull — and only `RefreshSystemAndFavorites` overwrites them
/// on conflict.
pub(super) fn upsert_session_row(
    tx: &rusqlite::Transaction,
    device_id: &str,
    sys: &SessionSystemData,
    favorited: bool,
    synced_group_id: &str,
    policy: SessionUpsertPolicy,
) -> AppResult<()> {
    tx.execute(
        &format!(
            "INSERT INTO sessions
             (id, device_id, source, project_dir, title_orig, started_at, last_active_at,
              agent_type, custom_title, favorited, synced_group_id, local_group_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(id, device_id) DO UPDATE SET {}",
            policy.conflict_set()
        ),
        params![
            sys.id,
            device_id,
            sys.source,
            sys.project_dir,
            sys.title_orig,
            sys.started_at,
            sys.last_active_at,
            sys.agent_type,
            "", // custom_title — device-local; neither caller carries it
            favorited as i64,
            synced_group_id,
            "", // local_group_id — device-private, never in git
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;
    use crate::model::SessionMessageRole;

    /// `upsert_session` (collect) uses `RefreshSystemOnly`: on conflict it
    /// refreshes the 6 system-data columns and MUST NOT touch the user-data
    /// columns — a re-extract preserves user edits. The regression test for the
    /// policy's system-only conflict set.
    #[test]
    fn upsert_session_refresh_system_only_preserves_user_data_on_reextract() {
        let store = mem();
        let dev = "0123456789ab";
        // First collect: creates the row with default user data.
        store
            .upsert_session(dev, &sys_session("s1", "2026-08-01T01:00:00.000Z"))
            .unwrap();
        // User edits: custom_title, favorited, local_group_id, synced_group_id.
        store
            .set_session_custom_title(dev, "s1", Some("Renamed"))
            .unwrap();
        store.set_session_favorited(dev, "s1", true).unwrap();
        store
            .set_session_local_group(dev, "s1", Some("lg1"))
            .unwrap();
        store
            .set_session_synced_group(dev, "s1", Some("sg1"))
            .unwrap();
        // Re-extract (next collect): system data refresh, must NOT clobber edits.
        store
            .upsert_session(dev, &sys_session("s1", "2026-08-02T09:00:00.000Z"))
            .unwrap();
        let m = store
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .find(|r| r.id == "s1")
            .unwrap();
        assert_eq!(
            m.last_active_at, "2026-08-02T09:00:00.000Z",
            "system refreshed"
        );
        assert_eq!(
            m.title, "Renamed",
            "custom_title preserved (title = custom_title)"
        );
        assert!(m.favorited, "favorited preserved");
        assert_eq!(m.synced_group_id, "sg1", "synced_group_id preserved");
        assert_eq!(m.local_group_id, "lg1", "local_group_id preserved");
    }

    #[test]
    fn query_sessions_time_range_filters_last_active_at() {
        let s = mem();
        seed_session(&s, "old", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "mid", "dev", "2026-08-15T10:00:00.000Z");
        seed_session(&s, "new", "dev", "2026-08-31T10:00:00.000Z");

        // from_ts narrows to sessions at or after Aug 10.
        let from = SessionFilter {
            from_ts: Some("2026-08-10T00:00:00.000Z".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&from))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["new", "mid"], "from_ts excludes early sessions");

        // to_ts narrows to sessions at or before Aug 20.
        let to = SessionFilter {
            to_ts: Some("2026-08-20T23:59:59.999Z".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&to))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["mid", "old"], "to_ts excludes late sessions");

        // both bounds → only "mid".
        let both = SessionFilter {
            from_ts: Some("2026-08-10T00:00:00.000Z".into()),
            to_ts: Some("2026-08-20T23:59:59.999Z".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&both))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["mid"], "from_ts + to_ts intersect to one session");
    }

    #[test]
    fn query_sessions_model_filter_uses_exists_semantics() {
        let s = mem();
        // s1 uses model A + B; s2 uses only B.
        seed_session_with_record(&s, "s1", "dev", "model-a");
        seed_session_with_record(&s, "s1", "dev", "model-b");
        seed_session_with_record(&s, "s2", "dev", "model-b");

        let ids = |model: &str| -> Vec<String> {
            let f = SessionFilter {
                model: Some(model.into()),
                ..Default::default()
            };
            s.query_sessions(Some(&f))
                .unwrap()
                .into_iter()
                .map(|r| r.id)
                .collect()
        };
        assert_eq!(ids("model-a"), ["s1"], "A matches only s1");
        let both: std::collections::BTreeSet<String> = ids("model-b").into_iter().collect();
        assert_eq!(
            both,
            std::collections::BTreeSet::from(["s1".to_string(), "s2".to_string()]),
            "B matches both (same last_active_at ⇒ order is unspecified)"
        );
        assert!(
            ids("no-such-model").is_empty(),
            "a model nobody used matches nothing"
        );
    }

    #[test]
    fn query_sessions_model_filter_is_device_isolated() {
        let s = mem();
        // Same session id on two devices; the model record exists only on dev1.
        seed_session_with_record(&s, "same", "dev1", "model-x");
        seed_session(&s, "same", "dev2", "2026-08-15T10:00:00.000Z");

        let f = SessionFilter {
            device_scope: Some("dev2".into()),
            model: Some("model-x".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions(Some(&f))
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(
            ids.is_empty(),
            "dev2's row must not match dev1's usage record (session ids can collide across devices)"
        );
    }

    /// `bulk_unfavorite_sessions` clears the favorited flag and deletes shared
    /// messages for exactly the given ids, in one transaction — leaving other
    /// favorited sessions and their messages untouched. `favorited_session_ids`
    /// feeds it by listing who is currently favorited. Empty input is a no-op.
    #[test]
    fn bulk_unfavorite_clears_flag_and_messages_for_the_given_ids() {
        let s = mem();
        // Three favorited sessions on a peer, each with one message.
        for sid in ["s1", "s2", "s3"] {
            seed_session(&s, sid, "peer", "2026-08-01T10:00:00.000Z");
            s.set_session_favorited("peer", sid, true).unwrap();
            s.ingest_session_messages_marking_dirty(
                "peer",
                std::slice::from_ref(&msg(
                    &format!("u-{sid}"),
                    sid,
                    SessionMessageRole::User,
                    "2026-08-01T10:00:00Z",
                )),
            )
            .unwrap();
        }
        assert_eq!(
            s.favorited_session_ids("peer").unwrap(),
            vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
            "lists all favorited, sorted"
        );

        // Un-favorite s2 only (its snapshot file "vanished" this pull).
        s.bulk_unfavorite_sessions("peer", &["s2".to_string()])
            .unwrap();
        assert_eq!(
            s.favorited_session_ids("peer").unwrap(),
            vec!["s1".to_string(), "s3".to_string()],
            "s2 cleared; s1/s3 kept"
        );
        assert!(
            s.query_session_messages("peer", "s2").unwrap().is_empty(),
            "s2 shared messages dropped"
        );
        assert_eq!(
            s.query_session_messages("peer", "s1").unwrap().len(),
            1,
            "untouched session keeps its message"
        );

        // Empty set is a no-op (no transaction, no error).
        assert_eq!(s.bulk_unfavorite_sessions("peer", &[]).unwrap(), 0);
    }

    // ---- reconcile_sessions (ghost-session reality check) ----

    #[test]
    fn reconcile_deletes_ghosts_keeps_seen_and_user_data() {
        let s = mem();
        seed_session(&s, "real", "dev", "2026-08-15T10:00:00.000Z");
        seed_session(&s, "ghost", "dev", "2026-08-10T10:00:00.000Z");
        // User data on the survivor must survive reconciliation.
        s.set_session_custom_title("dev", "real", Some("Renamed"))
            .unwrap();
        s.set_session_favorited("dev", "real", true).unwrap();
        s.set_session_local_group("dev", "real", Some("lg1"))
            .unwrap();

        // Messages for both sessions; the ghost's messages must be dropped in
        // the same transaction as its row — a session and its transcript are
        // one unit, never split.
        s.ingest_session_messages_marking_dirty(
            "dev",
            &[
                msg(
                    "u-real",
                    "real",
                    SessionMessageRole::User,
                    "2026-08-15T10:00:00Z",
                ),
                msg(
                    "u-ghost",
                    "ghost",
                    SessionMessageRole::User,
                    "2026-08-10T10:00:00Z",
                ),
            ],
        )
        .unwrap();

        let ghosts = s
            .reconcile_sessions("dev", "claude_code", &["real".to_string()])
            .unwrap();
        assert_eq!(ghosts, ["ghost"], "ghost row deleted, real row kept");

        let rows = s.query_sessions(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "real");
        assert_eq!(rows[0].title, "Renamed", "custom_title preserved");
        assert!(rows[0].favorited, "favorited preserved");
        assert_eq!(rows[0].local_group_id, "lg1", "group preserved");

        assert_eq!(
            s.query_session_messages("dev", "real").unwrap().len(),
            1,
            "survivor's messages kept"
        );
        assert!(
            s.query_session_messages("dev", "ghost").unwrap().is_empty(),
            "ghost's messages dropped with its row"
        );
    }

    #[test]
    fn reconcile_is_scoped_by_device_and_source() {
        let s = mem();
        seed_session(&s, "same", "dev1", "2026-08-15T10:00:00.000Z");
        seed_session(&s, "same", "dev2", "2026-08-15T10:00:00.000Z");
        // Another session id under a different source on the same device.
        seed_session_source(
            &s,
            "codex-same",
            "dev1",
            "codex_cli",
            "2026-08-15T10:00:00.000Z",
        );

        // Reconcile dev1/claude_code with nothing seen → dev1's claude row
        // goes, dev2's row and the codex row stay.
        let ghosts = s.reconcile_sessions("dev1", "claude_code", &[]).unwrap();
        assert!(ghosts.is_empty(), "empty seen set is a no-op");
        let ghosts = s
            .reconcile_sessions("dev1", "claude_code", &["other".to_string()])
            .unwrap();
        assert_eq!(ghosts, ["same"], "dev1 claude row is the ghost");

        let survivors: std::collections::BTreeSet<String> = s
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .map(|r| format!("{}/{}", r.device_id, r.source))
            .collect();
        assert_eq!(
            survivors,
            std::collections::BTreeSet::from([
                "dev2/claude_code".to_string(),
                "dev1/codex_cli".to_string(),
            ]),
            "peer + other-source rows untouched"
        );
    }

    #[test]
    fn reconcile_is_idempotent_and_empty_seen_is_noop() {
        let s = mem();
        seed_session(&s, "a", "dev", "2026-08-15T10:00:00.000Z");
        seed_session(&s, "b", "dev", "2026-08-15T10:00:00.000Z");
        // Empty seen → nothing deleted (protects a transiently invisible dir).
        assert!(s
            .reconcile_sessions("dev", "claude_code", &[])
            .unwrap()
            .is_empty());
        assert_eq!(s.query_sessions(None).unwrap().len(), 2);
        // First pass deletes the ghost.
        assert_eq!(
            s.reconcile_sessions("dev", "claude_code", &["a".to_string()])
                .unwrap(),
            ["b"]
        );
        // Second pass: nothing left to delete.
        assert!(s
            .reconcile_sessions("dev", "claude_code", &["a".to_string()])
            .unwrap()
            .is_empty());
        assert_eq!(s.query_sessions(None).unwrap().len(), 1);
    }
}
