//! Sessions table: per-session UPSERT + per-field user-data setters.
//!
//! Hosts the shared `upsert_session_row` / `SessionUpsertPolicy` core used by
//! both collect ([`Store::upsert_session`]) and pull
//! ([`super::transcript::Store::import_session_snapshot`]), plus
//! [`PushTrack`] — the single authority for which `sessions` columns ride git
//! (and therefore which setters flag the session dirty for the push path).

use super::store_transcript::{mark_sessions_dirty, tx_mark_session_dirty};
use super::*;

// ---- push-track classification: which `sessions` columns ride git ----
//
// The word pair "favorites-track vs device-local" has exactly one definition
// in the store domain: the enum below. Setters name their column's track in
// one closing [`tx_apply_push_track`] line; the upsert / pull / delete docs
// here and in `store_transcript` point at it instead of re-arguing the split.

/// Which sync track a user-writable `sessions` column rides — the single
/// authority for the "favorites-track vs device-local" split, i.e. for which
/// session fields ride git and which stay device-private. Every single-column
/// setter names its column's track at its [`tx_apply_push_track`] line, so a
/// new setter must classify its column against this enum instead of copying
/// whatever the nearest sibling happens to do (the miscopy that would either
/// silently never reach git, or churn empty pushes forever).
///
/// Why a `GitTracked` write must flag dirty: only a favorited session produces
/// a git snapshot ("a snapshot file exists ⇔ the session is favorited" — see
/// `snapshot_policy`), and that snapshot's meta line carries exactly the
/// GitTracked columns. A write to one therefore changes what git must hold and
/// reaches git only through the next push's snapshot recompute — which runs
/// only for sessions flagged dirty. The flag lands in the SAME transaction as
/// the write ([`tx_mark_session_dirty`]) so a crash can never commit the
/// column change without its flag: unflagged, the change would sit local
/// forever (the same "miss git" failure same-tx marking guards against for
/// usage rows).
///
/// Why a `DeviceLocal` write must NOT flag dirty: those columns are never
/// serialized into a snapshot, so a flag could only send the push path
/// rewriting a snapshot this write cannot change — a fabricated empty push.
pub(super) enum PushTrack {
    /// `favorited` + `synced_group_id` — carried to peers by the snapshot's
    /// meta line; writes flag the session dirty same-tx.
    GitTracked,
    /// `custom_title` + `local_group_id` + `excluded` — device-private, never
    /// carried by a snapshot; writes never flag dirty.
    DeviceLocal,
}

/// Apply the [`PushTrack`] coupling of the column a setter just wrote, inside
/// the setter's transaction — the one closing line every single-column setter
/// ends with. `GitTracked` flags the session dirty in this same transaction
/// (the change rides the next push's snapshot recompute); `DeviceLocal` is a
/// deliberate no-op. Call it with the track of the column the UPDATE above it
/// just wrote — that pairing is the decision this module makes greppable.
fn tx_apply_push_track(
    tx: &rusqlite::Transaction,
    session_id: &str,
    track: PushTrack,
) -> AppResult<()> {
    match track {
        PushTrack::GitTracked => tx_mark_session_dirty(tx, session_id),
        PushTrack::DeviceLocal => Ok(()),
    }
}

impl super::Store {
    // ---------------- Sessions ----------------

    /// Refresh a session's SYSTEM-data columns only. On conflict (same id +
    /// device_id), the [`SessionUpsertPolicy::RefreshSystemOnly`] clause
    /// updates exactly the refreshable columns (source / project_dir /
    /// title_orig / started_at / last_active_at / agent_type /
    /// parent_session_id) — it MUST NOT touch `custom_title` / `favorited` /
    /// `synced_group_id` / `local_group_id` / `excluded`. This is the
    /// SQLite-side encoding of the "re-extract never overwrites user data"
    /// invariant; a regression test in this module pins it.
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

    /// Set a session's favorited flag (user action). `PushTrack::GitTracked` —
    /// see the enum for why the closing line flags the session dirty.
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
        tx_apply_push_track(&tx, session_id, PushTrack::GitTracked)?;
        tx.commit()?;
        Ok(())
    }

    /// Set/clear a session's custom title. `None` or empty clears it (reverts
    /// to `title_orig` for display). `PushTrack::DeviceLocal` — see the enum
    /// for why this setter never flags dirty.
    pub fn set_session_custom_title(
        &self,
        device_id: &str,
        session_id: &str,
        title: Option<&str>,
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET custom_title = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, title.unwrap_or("").trim()],
        )?;
        tx_apply_push_track(&tx, session_id, PushTrack::DeviceLocal)?;
        tx.commit()?;
        Ok(())
    }

    /// Set/clear a session's local group (device-private).
    /// `PushTrack::DeviceLocal` — see the enum for why this setter never flags
    /// dirty.
    pub fn set_session_local_group(
        &self,
        device_id: &str,
        session_id: &str,
        group_id: Option<&str>,
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET local_group_id = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, group_id.unwrap_or("")],
        )?;
        tx_apply_push_track(&tx, session_id, PushTrack::DeviceLocal)?;
        tx.commit()?;
        Ok(())
    }

    /// Set/clear a session's synced group (cross-device).
    /// `PushTrack::GitTracked` — see the enum for why the closing line flags
    /// the session dirty.
    pub fn set_session_synced_group(
        &self,
        device_id: &str,
        session_id: &str,
        group_id: Option<&str>,
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE sessions SET synced_group_id = ?3 WHERE id = ?1 AND device_id = ?2",
            params![session_id, device_id, group_id.unwrap_or("")],
        )?;
        tx_apply_push_track(&tx, session_id, PushTrack::GitTracked)?;
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
    ///
    /// Ghosts are deleted outright, not dirty-flagged: a dirty flag drives the
    /// push path's materialization of a LIVE row, and a ghost has none. The
    /// git-side consequence (a formerly favorited ghost's snapshot file must
    /// vanish) is discharged synchronously by the collect caller via
    /// `decide_snapshot_action(false)` (best-effort unlink).
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

    /// Batch soft-delete: set the `excluded` marker on the given sessions so
    /// every sessions-side read (`build_session_where` filters `excluded = 0`)
    /// stops surfacing them. Soft by design — the app re-collects from the
    /// source session files, so a physical row delete would re-import on the
    /// next pass, and the user's source files must never be touched. The
    /// marker is device-private user data: no upsert conflict clause refreshes
    /// it, so neither a re-collect (`RefreshSystemOnly`) nor a peer-snapshot
    /// pull (`RefreshSystemAndFavorites`) can resurrect a deleted session.
    ///
    /// Deleting also clears `favorited` and flags every deleted id dirty in
    /// the SAME transaction. Clearing `favorited` is a
    /// [`PushTrack::GitTracked`] write driven by a LOCAL user action, so the
    /// same-tx flag is what makes the next push drop the sessions' git
    /// snapshots — a deleted session stops riding the favorites sync, and the
    /// "snapshot exists ⇔ favorited" invariant keeps holding (`excluded`
    /// itself is [`PushTrack::DeviceLocal`]: device-private, never
    /// serialized). This is the push-side mirror of a deletion; the pull-side
    /// mirror (`bulk_unfavorite_sessions`) deliberately does NOT flag dirty —
    /// an independent decision, see its doc. Transcript messages are kept (the
    /// exclusion is reversible in principle; messages are re-collectable
    /// derived data anyway). Returns how many rows matched
    /// (keys addressing no row simply don't count — a peer's row already
    /// reconciled away is not an error). Empty input is a no-op.
    pub fn delete_sessions(&self, keys: &[SessionKey]) -> AppResult<usize> {
        if keys.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        // The key set rides as a JSON array through json_each (the same
        // large-set pattern as reconcile), matched on the composite
        // (id, device_id) — a session is never addressable by bare id.
        let json = serde_json::to_string(keys)
            .map_err(|e| AppError::Internal(format!("delete sessions keys: {e}")))?;
        let n = tx.execute(
            "UPDATE sessions SET excluded = 1, favorited = 0 \
             WHERE (id, device_id) IN ( \
                SELECT json_extract(value, '$.id'), json_extract(value, '$.device_id') \
                FROM json_each(?1))",
            params![json],
        )?;
        let dirty: std::collections::BTreeSet<String> = keys.iter().map(|k| k.id.clone()).collect();
        mark_sessions_dirty(&tx, &dirty)?;
        tx.commit()?;
        Ok(n)
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
    /// Collect / re-extract: refresh the 7 system-data columns only. Never
    /// touches user-data columns — the "re-extract must not overwrite user
    /// edits" invariant (which is also what keeps a soft-deleted session
    /// excluded across every re-collect).
    RefreshSystemOnly,
    /// Pull / import: refresh the 7 system columns AND the two
    /// [`PushTrack::GitTracked`] columns — a peer's snapshot is authoritative
    /// for its own row's git-riding fields. The [`PushTrack::DeviceLocal`]
    /// columns stay untouched either way (never carried by a snapshot).
    RefreshSystemAndFavorites,
}

impl SessionUpsertPolicy {
    /// The `ON CONFLICT(id, device_id) DO UPDATE SET` clause this policy
    /// drives. Every policy refreshes the 7 shared system-data columns; pull
    /// additionally takes the two [`PushTrack::GitTracked`] columns. The
    /// [`PushTrack::DeviceLocal`] columns never appear here.
    fn conflict_set(&self) -> String {
        // The refreshable system-data columns — shared by both policies, so
        // declared once (single source of truth).
        const SYSTEM: &str = "source=excluded.source, project_dir=excluded.project_dir, \
                              title_orig=excluded.title_orig, started_at=excluded.started_at, \
                              last_active_at=excluded.last_active_at, \
                              agent_type=excluded.agent_type, \
                              parent_session_id=excluded.parent_session_id";
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
/// (pull). The INSERT lists all 14 columns (single source); on conflict,
/// `policy` picks which columns refresh. `custom_title`, `local_group_id` and
/// `excluded` seed as empty/off on INSERT for every caller (none is carried:
/// `custom_title` is a local edit, `local_group_id` never enters git, and
/// `excluded` — the soft-delete marker — is device-private user data no
/// collector may set). `favorited` / `synced_group_id` seed with the caller's
/// values — defaults for collect, the snapshot's for pull — and only
/// `RefreshSystemAndFavorites` overwrites them on conflict.
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
              agent_type, parent_session_id, custom_title, favorited, synced_group_id,
              local_group_id, excluded)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
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
            sys.parent_session_id,
            "", // custom_title — device-local; neither caller carries it
            favorited as i64,
            synced_group_id,
            "", // local_group_id — device-private, never in git
            0,  // excluded — device-private soft-delete marker; collect never sets it
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

    /// The [`PushTrack`] rule at the setter level — the regression pin for the
    /// "which session fields ride git" decision. GitTracked writes
    /// (favorited / synced_group) flag the session dirty in their transaction
    /// (the change must ride the next push's snapshot recompute); device-local
    /// writes (custom_title / local_group) leave the dirty set untouched (they
    /// cannot change a snapshot — flagging would fabricate an empty push).
    #[test]
    fn setter_dirty_marking_follows_push_track() {
        let s = mem();
        seed_session(&s, "local", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "fav-sid", "dev", "2026-08-02T10:00:00.000Z");
        seed_session(&s, "grp-sid", "dev", "2026-08-03T10:00:00.000Z");
        assert!(s.dirty_sessions().unwrap().is_empty(), "seeding is clean");

        // DeviceLocal: written, never flagged.
        s.set_session_custom_title("dev", "local", Some("Renamed"))
            .unwrap();
        s.set_session_local_group("dev", "local", Some("lg1"))
            .unwrap();
        assert!(
            s.dirty_sessions().unwrap().is_empty(),
            "device-local setters never flag dirty"
        );

        // GitTracked: each write flags its session dirty same-tx.
        s.set_session_favorited("dev", "fav-sid", true).unwrap();
        assert!(
            s.dirty_sessions().unwrap().contains(&"fav-sid".to_string()),
            "favorited flags dirty"
        );
        s.set_session_synced_group("dev", "grp-sid", Some("sg1"))
            .unwrap();
        assert!(
            s.dirty_sessions().unwrap().contains(&"grp-sid".to_string()),
            "synced_group flags dirty"
        );
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

    // ---------------------------------------------------------- paging ------

    /// Paged reads return consecutive time-desc slices with no overlap or gap
    /// (the ORDER BY tiebreakers make the ordering total) and the page sizes
    /// agree with the count query's total under the same filter.
    #[test]
    fn query_sessions_page_is_consecutive_and_agrees_with_count() {
        let s = mem();
        seed_session(&s, "d", "dev", "2026-08-04T10:00:00.000Z");
        seed_session(&s, "a", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "c", "dev", "2026-08-03T10:00:00.000Z");
        seed_session(&s, "e", "dev", "2026-08-05T10:00:00.000Z");
        seed_session(&s, "b", "dev", "2026-08-02T10:00:00.000Z");

        let page1 = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 0,
            })
            .unwrap();
        let page2 = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 2,
            })
            .unwrap();
        let page3 = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 4,
            })
            .unwrap();
        let ids =
            |rows: Vec<SessionRow>| -> Vec<String> { rows.into_iter().map(|r| r.id).collect() };
        assert_eq!(ids(page1), ["e", "d"], "page 1 = newest two");
        assert_eq!(ids(page2), ["c", "b"], "page 2 = next two");
        assert_eq!(ids(page3), ["a"], "page 3 = the tail");
        // Offsets past the end return an empty page, never an error.
        let past = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 2,
                offset: 99,
            })
            .unwrap();
        assert!(past.is_empty());

        let counts = s.count_sessions(None, "local").unwrap();
        assert_eq!(counts.total, 5, "count total matches the paged set");
        let all = s.query_sessions(None).unwrap();
        assert_eq!(all.len(), 5, "unpaged read still returns everything");
    }

    /// 越界 limit 被夹紧（#66：会话分页原漏夹紧、直接透传 query.limit）：
    /// 0 → 1（不空翻整页），超大 → 1000（不一次物化全表）。
    #[test]
    fn query_sessions_page_clamps_out_of_range_limits() {
        let s = mem();
        seed_session(&s, "a", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "b", "dev", "2026-08-02T10:00:00.000Z");
        // 0 → 夹到 1（返回一行，不空翻整页也不报错）；超大 → 夹到 1000（全返回）。
        let zero = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 0,
                offset: 0,
            })
            .unwrap();
        assert_eq!(zero.len(), 1, "limit=0 夹到 1");
        let huge = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: u32::MAX,
                offset: 0,
            })
            .unwrap();
        assert_eq!(huge.len(), 2, "limit 超大夹到 1000，全部返回");
    }

    /// Search is backend-side (LIKE) so a paged result searches the whole set,
    /// not just the loaded page. Matches the display title (custom title wins)
    /// and the project path, case-insensitively.
    #[test]
    fn query_sessions_page_search_matches_title_project_and_custom_title() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "s1".into(),
                source: "claude_code".into(),
                project_dir: "/home/u/parser".into(),
                title_orig: "Refactor tokenizer".into(),
                ..sys_session("s1", "2026-08-01T10:00:00.000Z")
            },
        )
        .unwrap();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "s2".into(),
                project_dir: "/home/u/www".into(),
                title_orig: "Unrelated".into(),
                ..sys_session("s2", "2026-08-02T10:00:00.000Z")
            },
        )
        .unwrap();

        let ids = |q: &str| -> Vec<String> {
            let filter = SessionFilter {
                search: Some(q.into()),
                ..Default::default()
            };
            s.query_sessions_page(&SessionQuery {
                filter: Some(filter),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
        };
        assert_eq!(ids("refactor"), ["s1"], "title_orig matches");
        assert_eq!(ids("parser"), ["s1"], "project_dir matches");
        assert!(ids("HELLO").is_empty(), "no match for unrelated text");
        // Custom title replaces the display title — search then sees it, not
        // the title_orig behind it (same COALESCE as the SELECT).
        s.set_session_custom_title("dev", "s1", Some("Casework"))
            .unwrap();
        assert_eq!(
            ids("casework"),
            ["s1"],
            "custom title becomes the searchable display title"
        );
        assert!(
            ids("refactor").is_empty(),
            "title_orig behind a custom title is not searched"
        );
        assert!(ids("zzz").is_empty(), "no match");
        // Search composes with the tab filter (device scope).
        let scoped = SessionFilter {
            device_scope: Some("other-dev".into()),
            search: Some("refactor".into()),
            ..Default::default()
        };
        assert!(s
            .query_sessions_page(&SessionQuery {
                filter: Some(scoped),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .is_empty());
    }

    /// LIKE wildcards in the search query are escaped — a literal `%` or `_`
    /// matches itself, mirroring the old client-side substring filter.
    #[test]
    fn query_sessions_page_search_escapes_like_wildcards() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "pct".into(),
                title_orig: "100% done".into(),
                ..sys_session("pct", "2026-08-01T10:00:00.000Z")
            },
        )
        .unwrap();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "plain".into(),
                title_orig: "One hundred".into(),
                ..sys_session("plain", "2026-08-02T10:00:00.000Z")
            },
        )
        .unwrap();
        let ids = |q: &str| -> Vec<String> {
            let filter = SessionFilter {
                search: Some(q.into()),
                ..Default::default()
            };
            s.query_sessions_page(&SessionQuery {
                filter: Some(filter),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
        };
        assert_eq!(ids("%"), ["pct"], "a lone % matches the literal % row only");
        assert_eq!(ids("00%"), ["pct"], "% is not a wildcard in the query");
        assert_eq!(
            ids("One hundred"),
            ["plain"],
            "plain query unaffected by escaping"
        );
    }

    // ---- delete_sessions (soft delete / exclusion marker, #91) ----

    /// Soft delete hides the sessions from EVERY sessions-side read (the list,
    /// the counts, the stats dimension, and the jump-channel row read) while
    /// leaving other rows and the physical rows themselves untouched.
    #[test]
    fn delete_sessions_hides_rows_from_sessions_reads() {
        let s = mem();
        seed_session(&s, "keep", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "gone", "dev", "2026-08-02T10:00:00.000Z");
        s.set_session_favorited("dev", "gone", true).unwrap();

        let n = s
            .delete_sessions(&[SessionKey {
                id: "gone".into(),
                device_id: "dev".into(),
            }])
            .unwrap();
        assert_eq!(n, 1, "one row matched");

        let ids: Vec<String> = s
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["keep"], "list drops the deleted session");
        assert_eq!(
            s.count_sessions(None, "local").unwrap().total,
            1,
            "counts drop it too"
        );
        assert_eq!(
            s.query_session_stats(None).unwrap().len(),
            1,
            "the stats dimension drops it too"
        );
        assert!(
            s.get_session("gone", "dev").unwrap().is_none(),
            "the jump channel treats deleted as nonexistent"
        );
        assert!(
            s.get_session("keep", "dev").unwrap().is_some(),
            "survivors still resolve"
        );
        // Composite-key addressing: a same-id row on another device is not
        // touched, and a key matching no row simply doesn't count.
        seed_session(&s, "gone", "peer", "2026-08-03T10:00:00.000Z");
        assert_eq!(
            s.delete_sessions(&[SessionKey {
                id: "no-such".into(),
                device_id: "dev".into()
            }])
            .unwrap(),
            0,
            "no row matched"
        );
        assert!(
            s.get_session("gone", "peer").unwrap().is_some(),
            "the peer's row survives a dev-scoped delete"
        );
        // Empty input is a no-op.
        assert_eq!(s.delete_sessions(&[]).unwrap(), 0);
    }

    /// The acceptance invariant (#91): after a delete, neither a re-collect
    /// (RefreshSystemOnly upsert) nor a peer-snapshot pull
    /// (RefreshSystemAndFavorites upsert) resurrects the session — the
    /// `excluded` marker rides no conflict clause. The same-tx dirty marking
    /// also lands, so the next push drops the deleted favorite's snapshot.
    #[test]
    fn delete_sessions_marker_survives_recollect_and_pull() {
        let s = mem();
        seed_session(&s, "sx", "dev", "2026-08-01T10:00:00.000Z");
        s.set_session_favorited("dev", "sx", true).unwrap();
        s.delete_sessions(&[SessionKey {
            id: "sx".into(),
            device_id: "dev".into(),
        }])
        .unwrap();
        assert!(
            s.dirty_sessions().unwrap().contains(&"sx".to_string()),
            "delete flags the session dirty so the push drops its snapshot"
        );

        // Re-collect re-offers the same system data (RefreshSystemOnly).
        s.upsert_session("dev", &sys_session("sx", "2026-08-09T00:00:00.000Z"))
            .unwrap();
        // Pull re-offers the peer's snapshot (RefreshSystemAndFavorites — even
        // with favorited=true, the peer's own view).
        s.import_session_snapshot(
            "dev",
            &SessionSnapshotMeta {
                v: SESSION_SNAPSHOT_VERSION,
                id: "sx".into(),
                source: "claude_code".into(),
                project_dir: "/proj".into(),
                title_orig: "Title".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-09T00:00:00.000Z".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
                favorited: true,
                synced_group_id: String::new(),
            },
            &[],
        )
        .unwrap();

        // The row physically exists (system data refreshed, favorited re-set by
        // the peer's authoritative snapshot) but stays hidden: `excluded` was
        // never in any conflict set.
        let conn = s.conn.lock().expect("db mutex poisoned");
        let (excluded, favorited, last): (i64, i64, String) = conn
            .query_row(
                "SELECT excluded, favorited, last_active_at FROM sessions \
                 WHERE id = 'sx' AND device_id = 'dev'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        drop(conn);
        assert_eq!(excluded, 1, "the marker survived both upsert paths");
        assert_eq!(
            favorited, 1,
            "the peer's snapshot stays authoritative for favorites"
        );
        assert_eq!(last, "2026-08-09T00:00:00.000Z", "system data refreshed");
        assert!(
            s.query_sessions(None).unwrap().is_empty(),
            "still invisible to the sessions list"
        );
    }

    /// Deleting clears `favorited` in the same transaction — a deleted session
    /// stops riding the favorites sync (the push path then removes its git
    /// snapshot per the snapshot-exists ⇔ favorited rule).
    #[test]
    fn delete_sessions_clears_favorited_in_same_transaction() {
        let s = mem();
        seed_session(&s, "fav", "dev", "2026-08-01T10:00:00.000Z");
        s.set_session_favorited("dev", "fav", true).unwrap();
        seed_session(&s, "plain", "dev", "2026-08-02T10:00:00.000Z");
        s.delete_sessions(&[
            SessionKey {
                id: "fav".into(),
                device_id: "dev".into(),
            },
            SessionKey {
                id: "plain".into(),
                device_id: "dev".into(),
            },
        ])
        .unwrap();
        assert_eq!(
            s.favorited_session_ids("dev").unwrap(),
            Vec::<String>::new(),
            "deleted favorites leave the favorites list"
        );
    }

    /// Parent link roundtrip (#90): a subagent row's `parent_session_id`
    /// persists and crosses to the list DTO, keyed to the same device.
    #[test]
    fn session_list_carries_parent_link() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "main-1".into(),
                agent_type: String::new(),
                parent_session_id: String::new(),
                ..sys_session("main-1", "2026-08-02T10:00:00.000Z")
            },
        )
        .unwrap();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "agent-x".into(),
                agent_type: "Explore".into(),
                parent_session_id: "main-1".into(),
                ..sys_session("agent-x", "2026-08-01T10:00:00.000Z")
            },
        )
        .unwrap();
        let rows = s.query_sessions(None).unwrap();
        let by_id = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(by_id("agent-x").agent_type, "Explore");
        assert_eq!(by_id("agent-x").parent_session_id, "main-1");
        assert_eq!(by_id("main-1").parent_session_id, "");
    }

    /// Sidebar counts: total under the filter + one bucket per distinct group
    /// column value (empty string = ungrouped), per track.
    #[test]
    fn count_sessions_totals_and_group_buckets_per_track() {
        let s = mem();
        for (id, last) in [
            ("a", "2026-08-01T10:00:00.000Z"),
            ("b", "2026-08-02T10:00:00.000Z"),
            ("c", "2026-08-03T10:00:00.000Z"),
            ("d", "2026-08-04T10:00:00.000Z"),
        ] {
            seed_session(&s, id, "dev", last);
        }
        s.set_session_local_group("dev", "a", Some("lg1")).unwrap();
        s.set_session_local_group("dev", "b", Some("lg1")).unwrap();
        s.set_session_local_group("dev", "c", Some("lg2")).unwrap();
        s.set_session_synced_group("dev", "a", Some("sg1")).unwrap();

        let local = s.count_sessions(None, "local").unwrap();
        assert_eq!(local.total, 4, "total ignores the track");
        let buckets: std::collections::BTreeMap<String, u32> = local
            .groups
            .iter()
            .map(|g| (g.group_id.clone(), g.count))
            .collect();
        assert_eq!(buckets["lg1"], 2, "two sessions in lg1");
        assert_eq!(buckets["lg2"], 1, "one session in lg2");
        assert_eq!(buckets[""], 1, "the ungrouped bucket is the empty id");

        let synced = s.count_sessions(None, "synced").unwrap();
        let synced_buckets: std::collections::BTreeMap<String, u32> = synced
            .groups
            .iter()
            .map(|g| (g.group_id.clone(), g.count))
            .collect();
        assert_eq!(synced_buckets["sg1"], 1);
        assert_eq!(synced_buckets[""], 3);

        // Filtered counts narrow with the filter (source scope).
        let src_filter = SessionFilter {
            source: Some("codex_cli".into()),
            ..Default::default()
        };
        let empty = s.count_sessions(Some(&src_filter), "local").unwrap();
        assert_eq!(empty.total, 0);
        assert!(empty.groups.is_empty());

        // Unknown track is a hard error, not a silent wrong-column read.
        assert!(s.count_sessions(None, "bogus").is_err());
    }
}
