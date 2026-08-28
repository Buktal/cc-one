//! `sessions` 表写路径 + 同步耦合：per-session UPSERT、per-field 用户数据
//! setter、dirty 核心、pull 侧快照导入与收藏对账。
//!
//! 「写路径」归这里的不只是 UPDATE/INSERT：把一条写送上 git 的整套耦合也在这
//! —— [`PushTrack`]（哪列进快照）、dirty 核心
//! （[`tx_mark_session_dirty`] / [`mark_sessions_dirty`]，同事务标脏规则）、
//! push 物化路径的输入读 [`Store::get_session_snapshot_meta`] 与
//! [`Store::dirty_sessions`]（读的是 push 路径的输入，随 push 耦合同住，不随
//! 读路径住 `store_sessions_reads`）。会话列表/计数的读路径在
//! `super::store_sessions_reads`；消息本体（`session_messages` 表）在
//! `super::store_transcript`。
//!
//! Hosts the shared `upsert_session_row` / `SessionUpsertPolicy` core used by
//! both collect ([`Store::upsert_session`]) and pull
//! ([`Store::import_session_snapshot`]), plus [`PushTrack`] — the single
//! authority for which `sessions` columns ride git (and therefore which setters
//! flag the session dirty for the push path).

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

/// Flag ONE session dirty inside `tx` — the same-tx core of the whole
/// dirty-session mechanism. The flag must land atomically with the write that
/// made the session dirty (a separate transaction could leave a committed
/// write whose session is never flagged, silently dropping it from the next
/// push — the same failure `mark_days_dirty` guards against for usage rows).
/// `INSERT OR IGNORE` keeps re-flagging idempotent. Setter-path callers go
/// through [`tx_apply_push_track`], which applies this only for
/// `PushTrack::GitTracked` columns.
pub(super) fn tx_mark_session_dirty(tx: &rusqlite::Transaction, session_id: &str) -> AppResult<()> {
    tx.execute(
        "INSERT OR IGNORE INTO dirty_sessions(session_id) VALUES (?1)",
        params![session_id],
    )?;
    Ok(())
}

/// Flag each session in `sessions` dirty within `tx` — the batch form of
/// [`tx_mark_session_dirty`] for the multi-row writers (transcript ingest,
/// `delete_sessions`), so every flag still lands in the writer's transaction.
pub(super) fn mark_sessions_dirty(
    tx: &rusqlite::Transaction,
    sessions: &std::collections::BTreeSet<String>,
) -> AppResult<()> {
    for sid in sessions {
        tx_mark_session_dirty(tx, sid)?;
    }
    Ok(())
}

/// Recompute-time message count for one session — what the push wrote.
/// `Store::clear_dirty_flags_if_unchanged` re-checks it before dropping the
/// session's dirty flag, so a message that raced in after the snapshot keeps
/// the session dirty (a blind delete would strand it on the local-only side
/// of git forever).
pub struct SessionCounts {
    pub session_id: String,
    pub message_rows: usize,
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
    /// every sessions-side read (`build_session_where` in
    /// `store_sessions_reads` filters `excluded = 0`) stops surfacing them.
    /// Soft by design — the app re-collects from the source session files, so
    /// a physical row delete would re-import on the next pass, and the user's
    /// source files must never be touched. The marker is device-private user
    /// data: no upsert conflict clause refreshes it, so neither a re-collect
    /// (`RefreshSystemOnly`) nor a peer-snapshot pull
    /// (`RefreshSystemAndFavorites`) can resurrect a deleted session.
    ///
    /// Deleting also clears `favorited` and flags every deleted id dirty in
    /// the SAME transaction. Clearing `favorited` is a
    /// [`PushTrack::GitTracked`] write driven by a LOCAL user action, so the
    /// same-tx flag is what makes the next push drop the sessions' git
    /// snapshots — a deleted session stops riding the favorites sync, and the
    /// "snapshot exists ⇔ favorited" invariant keeps holding (`excluded`
    /// itself is [`PushTrack::DeviceLocal`]: device-private, never
    /// serialized). This is the push-side mirror of a deletion; the pull-side
    /// mirror ([`Store::bulk_unfavorite_sessions`]) deliberately does NOT flag
    /// dirty — an independent decision, see its doc. Transcript messages are
    /// kept (the exclusion is reversible in principle; messages are
    /// re-collectable derived data anyway). Returns how many rows matched
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

    /// The session ids holding un-pushed changes, in deterministic order (sorted).
    /// Drives the push path's per-session jsonl recompute (a recompute WRITES the
    /// snapshot for a favorited session, DELETES it for a non-favorited one).
    /// Read-only — it does NOT clear: clearing happens only after a push lands
    /// (`Store::clear_dirty_flags_if_unchanged`), so a failed push retries on
    /// the next attempt. Pure local state: makes no claim about the git worktree.
    pub fn dirty_sessions(&self) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare("SELECT session_id FROM dirty_sessions ORDER BY session_id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// The meta line the push path writes as the first line of a session's jsonl
    /// snapshot, read straight from the sessions row (system data + the two
    /// `PushTrack::GitTracked` user fields — the column-track authority is
    /// [`PushTrack`]). `None` when the session is not in the table — the
    /// caller treats that as "nothing to snapshot".
    pub fn get_session_snapshot_meta(
        &self,
        device_id: &str,
        session_id: &str,
    ) -> AppResult<Option<SessionSnapshotMeta>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let row = conn
            .query_row(
                "SELECT id, source, project_dir, title_orig, started_at, last_active_at,
                        agent_type, parent_session_id, favorited, synced_group_id
                 FROM sessions WHERE id = ?1 AND device_id = ?2",
                params![session_id, device_id],
                |r| {
                    Ok(SessionSnapshotMeta {
                        v: SESSION_SNAPSHOT_VERSION,
                        id: r.get(0)?,
                        source: r.get(1)?,
                        project_dir: r.get(2)?,
                        title_orig: r.get(3)?,
                        started_at: r.get(4)?,
                        last_active_at: r.get(5)?,
                        agent_type: r.get(6)?,
                        parent_session_id: r.get(7)?,
                        favorited: r.get::<_, i64>(8)? != 0,
                        synced_group_id: r.get(9)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Import a peer's session snapshot into the store (pull path). UPSERTs the
    /// session row keyed by `(meta.id, device_id)`; the peer's `favorited` and
    /// `synced_group_id` (carried by the snapshot) overwrite — the peer is
    /// authoritative for its own row's `PushTrack::GitTracked` columns. System
    /// fields refresh on conflict; the `PushTrack::DeviceLocal` columns
    /// (`custom_title` / `local_group_id`) are never carried by the snapshot.
    /// Messages land deduped by `(device_id, uuid)`, NOT marked dirty (pull
    /// data is already on git — the same split as `ingest` vs
    /// `ingest_marking_dirty` for usage rows).
    pub fn import_session_snapshot(
        &self,
        device_id: &str,
        meta: &SessionSnapshotMeta,
        messages: &[SessionMessage],
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        // Project the snapshot meta into the system-data carrier (the 7 fields
        // a snapshot shares with a freshly collected session); the GitTracked
        // columns ride the builder args + RefreshSystemAndFavorites.
        upsert_session_row(
            &tx,
            device_id,
            &SessionSystemData {
                id: meta.id.clone(),
                source: meta.source.clone(),
                project_dir: meta.project_dir.clone(),
                title_orig: meta.title_orig.clone(),
                started_at: meta.started_at.clone(),
                last_active_at: meta.last_active_at.clone(),
                agent_type: meta.agent_type.clone(),
                parent_session_id: meta.parent_session_id.clone(),
            },
            meta.favorited,
            &meta.synced_group_id,
            SessionUpsertPolicy::RefreshSystemAndFavorites,
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO session_messages
                 (device_id, session_id, uuid, role, ts, model, name, content)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT (device_id, uuid) DO NOTHING",
            )?;
            for m in messages {
                stmt.execute(params![
                    device_id,
                    m.session_id,
                    m.uuid,
                    m.role.as_str(),
                    m.ts,
                    m.model.as_deref().unwrap_or(""),
                    m.name.as_deref().unwrap_or(""),
                    m.content,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The peer devices that currently have a favorited session row, excluding
    /// self. Drives the un-favorite propagation's "which peers to reconcile"
    /// loop — a peer that ships NO snapshot files this pull still needs its rows
    /// checked, because it may have un-favorited everything.
    pub fn favorited_session_devices(&self, self_device_id: &str) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT DISTINCT device_id FROM sessions \
             WHERE favorited = 1 AND device_id != ?1 ORDER BY device_id",
        )?;
        let rows = stmt.query_map(params![self_device_id], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// The session ids this device currently has favorited. Paired with the
    /// still-present snapshot files via `snapshot_policy::presence_mismatches`
    /// to drive the pull-side un-favorite reconciliation. Sorted for
    /// deterministic reconciliation.
    pub fn favorited_session_ids(&self, device_id: &str) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id FROM sessions WHERE device_id = ?1 AND favorited = 1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![device_id], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Clear the favorited flag and drop shared transcript messages for a set
    /// of the device's sessions (pull path: these are sessions a peer
    /// un-favorited, detected by their snapshot file vanishing — the set is the
    /// `snapshot_policy::presence_mismatches` oracle's `favorites_without_files`
    /// half, already computed by the caller). One transaction: both writes land
    /// together or neither does; the meta row stays (its system data is
    /// harmless, and self may hold its own row for the same session). No-op
    /// (returns 0) on an empty set.
    ///
    /// NOT marked dirty — an independent decision that deliberately departs
    /// from the setter-path rule for a `PushTrack::GitTracked` column (see
    /// [`PushTrack`]): this write does not CREATE a local change needing a
    /// push, it MIRRORS one already on git (the peer removed its snapshot file
    /// — that vanishing is how these ids were selected). The dirty flag also
    /// keys by bare session id while the push materializes only SELF's rows,
    /// so flagging a peer-row write would at best trigger a meaningless
    /// recompute of self's row for the same id.
    pub fn bulk_unfavorite_sessions(&self, device_id: &str, ids: &[String]) -> AppResult<usize> {
        if ids.is_empty() {
            // Nothing to write; skip opening a transaction entirely.
            return Ok(0);
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        let json = serde_json::to_string(ids)
            .map_err(|e| AppError::Internal(format!("bulk unfavorite: {e}")))?;
        tx.execute(
            "UPDATE sessions SET favorited = 0 \
             WHERE device_id = ?1 AND id IN (SELECT value FROM json_each(?2))",
            params![device_id, json],
        )?;
        tx.execute(
            "DELETE FROM session_messages \
             WHERE device_id = ?1 AND session_id IN (SELECT value FROM json_each(?2))",
            params![device_id, json],
        )?;
        tx.commit()?;
        Ok(ids.len())
    }
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
            s.count_sessions(None, GroupTrack::Local).unwrap().total,
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

    /// The other policy half: `import_session_snapshot` (pull) uses
    /// `RefreshSystemAndFavorites`, so on conflict it overwrites the
    /// `PushTrack::GitTracked` columns (a peer is authoritative for its own
    /// row's favorited / synced_group_id) but leaves the
    /// `PushTrack::DeviceLocal` columns (custom_title, local_group_id)
    /// untouched.
    #[test]
    fn import_session_snapshot_refreshes_favorites_but_not_device_local() {
        let store = mem();
        let dev = "aabbccddeeff"; // a peer's device id

        // Local state: the row already exists with custom_title + local_group,
        // and the session is NOT favorited locally.
        store
            .upsert_session(dev, &sys_session("s1", "2026-08-01T01:00:00.000Z"))
            .unwrap();
        store
            .set_session_custom_title(dev, "s1", Some("LocalName"))
            .unwrap();
        store
            .set_session_local_group(dev, "s1", Some("lg-local"))
            .unwrap();

        // Peer imports a snapshot: favorited=true, synced_group=sg-peer, with
        // refreshed system data. Favorites-track columns follow the peer;
        // device-local columns survive.
        let meta = SessionSnapshotMeta {
            v: SESSION_SNAPSHOT_VERSION,
            id: "s1".into(),
            source: "claude_code".into(),
            project_dir: "/proj".into(),
            title_orig: "orig-title".into(),
            started_at: "2026-08-01T00:00:00.000Z".into(),
            last_active_at: "2026-08-02T09:00:00.000Z".into(),
            agent_type: "Explore".into(),
            parent_session_id: String::new(),
            favorited: true,
            synced_group_id: "sg-peer".into(),
        };
        store.import_session_snapshot(dev, &meta, &[]).unwrap();
        let m = store
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .find(|r| r.id == "s1")
            .unwrap();
        assert_eq!(
            m.last_active_at, "2026-08-02T09:00:00.000Z",
            "system refreshed from peer"
        );
        assert_eq!(
            m.agent_type, "Explore",
            "agent_type refreshed from peer snapshot (pull must not zero it)"
        );
        assert!(m.favorited, "favorited overwritten by peer");
        assert_eq!(
            m.synced_group_id, "sg-peer",
            "synced_group_id overwritten by peer"
        );
        assert_eq!(
            m.title, "LocalName",
            "custom_title preserved (device-local, not in snapshot)"
        );
        assert_eq!(
            m.local_group_id, "lg-local",
            "local_group_id preserved (device-private, never in git)"
        );
    }
}
