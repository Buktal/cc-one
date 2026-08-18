//! Session transcript: message 原文 ingest/reads, session queries, dirty-session
//! tracking, and the pull-side snapshot import + favorites reconciliation.
//!
//! Hosts the shared `mark_sessions_dirty` helper (used by the sessions-domain
//! favorites setters) and the `build_session_where` / `row_to_session_message`
//! decode helpers.

use super::store_sessions::{upsert_session_row, SessionUpsertPolicy};
use super::*;

/// Recompute-time message count for one session — what the push wrote.
/// `Store::clear_dirty_flags_if_unchanged` re-checks it before dropping the
/// session's dirty flag, so a message that raced in after the snapshot keeps
/// the session dirty (a blind delete would strand it on the local-only side
/// of git forever).
pub struct SessionCounts {
    pub session_id: String,
    pub message_rows: usize,
}

impl super::Store {
    // ---------------- Session messages (transcript 原文, db source of truth) ----

    /// Insert transcript messages for one device, deduping by `(device_id, uuid)`
    /// (ON CONFLICT DO NOTHING + RETURNING). Returns the newly inserted subset;
    /// the ingest layer marks exactly those messages' sessions dirty. ALL sessions
    /// land here (not just favorited) — SQLite is the single source of truth for
    /// message 原文, and only the derived `sessions/<id>.jsonl` snapshot is
    /// favorites-gated (push path). Mirrors `ingest_marking_dirty`: one
    /// transaction, RETURNING to detect real new rows, mark dirty in the same tx.
    pub fn ingest_session_messages_marking_dirty(
        &self,
        device_id: &str,
        messages: &[SessionMessage],
    ) -> AppResult<Vec<SessionMessage>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        let mut inserted: Vec<SessionMessage> = Vec::new();
        for m in messages {
            let landed: Option<String> = tx
                .query_row(
                    "INSERT INTO session_messages
                     (device_id, session_id, uuid, role, ts, model, name, content)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                     ON CONFLICT (device_id, uuid) DO NOTHING
                     RETURNING uuid",
                    params![
                        device_id,
                        m.session_id,
                        m.uuid,
                        m.role.as_str(),
                        m.ts,
                        m.model.as_deref().unwrap_or(""),
                        m.name.as_deref().unwrap_or(""),
                        m.content,
                    ],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if landed.is_some() {
                inserted.push(m.clone());
            }
        }
        if !inserted.is_empty() {
            let dirty: std::collections::BTreeSet<String> =
                inserted.iter().map(|m| m.session_id.clone()).collect();
            mark_sessions_dirty(&tx, &dirty)?;
        }
        tx.commit()?;
        Ok(inserted)
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
    /// favorites-track user fields). `None` when the session is not in the table
    /// — the caller treats that as "nothing to snapshot".
    pub fn get_session_snapshot_meta(
        &self,
        device_id: &str,
        session_id: &str,
    ) -> AppResult<Option<SessionSnapshotMeta>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let row = conn
            .query_row(
                "SELECT id, source, project_dir, title_orig, started_at, last_active_at,
                        agent_type, favorited, synced_group_id
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
                        favorited: r.get::<_, i64>(7)? != 0,
                        synced_group_id: r.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    /// Import a peer's session snapshot into the store (pull path). UPSERTs the
    /// session row keyed by `(meta.id, device_id)`; the peer's `favorited` and
    /// `synced_group_id` (carried by the snapshot) overwrite — the peer is
    /// authoritative for its own row's favorites-track fields. System fields
    /// refresh on conflict; `custom_title` / `local_group_id` are device-local
    /// and never carried by the snapshot. Messages land deduped by
    /// `(device_id, uuid)`, NOT marked dirty (pull data is already on git — the
    /// same split as `ingest` vs `ingest_marking_dirty` for usage rows).
    pub fn import_session_snapshot(
        &self,
        device_id: &str,
        meta: &SessionSnapshotMeta,
        messages: &[SessionMessage],
    ) -> AppResult<()> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        // Project the snapshot meta into the system-data carrier (the 7 fields
        // a snapshot shares with a freshly collected session); the
        // favorites-track fields ride the builder args + RefreshSystemAndFavorites.
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
    /// (returns 0) on an empty set. NOT marked dirty: a pull-side reconciliation
    /// is not a local change to push.
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

    /// Read one session's transcript messages for a device, in chronological
    /// order (`ts`, then `uuid` as a stable tiebreaker). The transcript read
    /// path's per-device lookup; the command layer merges across devices. The
    /// empty-string `model`/`name` stored for `None` round-trips back to `None`.
    pub fn query_session_messages(
        &self,
        device_id: &str,
        session_id: &str,
    ) -> AppResult<Vec<SessionMessage>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT uuid, session_id, role, ts, model, name, content \
             FROM session_messages \
             WHERE device_id = ?1 AND session_id = ?2 \
             ORDER BY ts, uuid",
        )?;
        let rows = stmt.query_map(params![device_id, session_id], row_to_session_message)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Read a session's transcript merged across ALL devices that hold it,
    /// deduped by uuid with `self_device_id` winning conflicts (it is the
    /// source of truth for a session it collected). The transcript read path
    /// (command layer); the favorites gate does NOT apply — every session's
    /// messages are in the db. Self's slice is read first so its uuids claim the
    /// dedup set; peers (alphabetic order) only fill gaps. The merged result is
    /// re-sorted by (ts, uuid) for a fully chronological transcript.
    pub fn query_session_transcript(
        &self,
        session_id: &str,
        self_device_id: &str,
    ) -> AppResult<Vec<SessionMessage>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut devices_stmt =
            conn.prepare("SELECT DISTINCT device_id FROM session_messages WHERE session_id = ?1")?;
        let mut devices: Vec<String> = devices_stmt
            .query_map(params![session_id], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(devices_stmt);
        // Self first (its uuids win), then peers in stable alphabetic order.
        devices.sort();
        if let Some(pos) = devices.iter().position(|d| d == self_device_id) {
            devices.swap(0, pos);
        }
        let mut stmt = conn.prepare(
            "SELECT uuid, session_id, role, ts, model, name, content \
             FROM session_messages \
             WHERE device_id = ?1 AND session_id = ?2",
        )?;
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut out: Vec<SessionMessage> = Vec::new();
        for did in &devices {
            let rows = stmt.query_map(params![did, session_id], row_to_session_message)?;
            for m in rows {
                let m = m?;
                if seen.insert(m.uuid.clone()) {
                    out.push(m);
                }
            }
        }
        // Each per-device slice arrives in storage order; re-sort the merged set
        // by (ts, uuid) so the cross-device transcript is chronological.
        out.sort_by(|a, b| (&a.ts, &a.uuid).cmp(&(&b.ts, &b.uuid)));
        Ok(out)
    }

    /// List sessions for the UI, joined live with `usage_records` to compute
    /// per-session request_count / total_tokens / total_cost_usd (the usage
    /// table is the single source of token truth). Title = `custom_title` when
    /// set, else `title_orig`. `filter` is optional; `None` lists every session.
    /// Unpaged — retained for test-only callers (the collector/sync tests);
    /// production reads go through [`Store::query_sessions_page`] so the UI
    /// only materializes one page.
    #[cfg(test)]
    pub fn query_sessions(&self, filter: Option<&SessionFilter>) -> AppResult<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let sql = sessions_select_sql(&clause);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), session_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// One page of the session list for the UI — same rows as
    /// [`Store::query_sessions`] but `LIMIT ? OFFSET ?` applied so a large
    /// session table renders a page instead of loading everything (mirrors the
    /// request-log table's paging). The ORDER BY adds `device_id`/`id`
    /// tiebreakers so pages never duplicate or skip rows across page turns.
    pub fn query_sessions_page(&self, query: &SessionQuery) -> AppResult<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, mut params_vec) = build_session_where(query.filter.as_ref());
        let sql = format!("{} LIMIT ? OFFSET ?", sessions_select_sql(&clause));
        params_vec.push(SqlValue::Integer(super::page_limit(query.limit)));
        params_vec.push(SqlValue::Integer(query.offset as i64));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), session_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Sidebar + paginator counts for one grouping track under a filter: the
    /// total session count (drives the paginator and the sidebar's "All" row)
    /// plus per-bucket counts (the group rows). The track selects the group
    /// column (`local` → `local_group_id`, `synced` → `synced_group_id`);
    /// every distinct column value becomes a bucket, including the empty
    /// string (ungrouped) and stale ids whose group was deleted — the client
    /// resolves those against its known group list. Paging-independent: it
    /// describes the whole filtered set.
    pub fn count_sessions(
        &self,
        filter: Option<&SessionFilter>,
        track: &str,
    ) -> AppResult<SessionGroupCounts> {
        let col = match track {
            "local" => "local_group_id",
            "synced" => "synced_group_id",
            other => return Err(AppError::Internal(format!("unknown group track: {other}"))),
        };
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM sessions s {clause}"),
            params_from_iter(params_vec.iter()),
            |r| r.get(0),
        )?;
        let sql = format!(
            "SELECT s.{col} AS gid, COUNT(*) AS n \
             FROM sessions s {clause} GROUP BY s.{col}"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            Ok(SessionGroupCount {
                group_id: r.get(0)?,
                count: r.get::<_, i64>(1)? as u32,
            })
        })?;
        let groups = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;
        Ok(SessionGroupCounts {
            total: total as u32,
            groups,
        })
    }
}

/// Decode a `session_messages` row in the canonical SELECT column order
/// (`uuid, session_id, role, ts, model, name, content`). Shared by the
/// per-device and cross-device transcript reads so the column↔field mapping
/// lives in one place — a positional drift here would silently misassign role
/// and ts (the bug the `ingest_session_messages_flags_dirty_for_all_sessions`
/// round-trip test catches).
fn row_to_session_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMessage> {
    let model: String = r.get(4)?;
    let name: String = r.get(5)?;
    Ok(SessionMessage {
        uuid: r.get(0)?,
        session_id: r.get(1)?,
        role: SessionMessageRole::parse_str(&r.get::<_, String>(2)?),
        ts: r.get(3)?,
        model: if model.is_empty() { None } else { Some(model) },
        name: if name.is_empty() { None } else { Some(name) },
        content: r.get(6)?,
    })
}

/// Flag each session in `sessions` dirty, within `tx` so the flag lands
/// atomically with the `session_messages` writes that made them dirty (a separate
/// transaction could leave a written message whose session is never flagged,
/// silently dropping it from the next push — the same failure `mark_days_dirty`
/// guards against for usage rows). `INSERT OR IGNORE` keeps it idempotent across
/// collects.
pub(super) fn mark_sessions_dirty(
    tx: &rusqlite::Transaction,
    sessions: &std::collections::BTreeSet<String>,
) -> AppResult<()> {
    if sessions.is_empty() {
        return Ok(());
    }
    let mut stmt = tx.prepare("INSERT OR IGNORE INTO dirty_sessions(session_id) VALUES (?1)")?;
    for sid in sessions {
        stmt.execute(params![sid])?;
    }
    Ok(())
}

/// Build a WHERE clause over the `sessions` table for a [`SessionFilter`]. The
/// clause prefixes every column with `s.` so it composes with the
/// `usage_records` subquery JOIN in [`Store::query_sessions`]. Empty filter ⇒
/// `("", [])`.
fn build_session_where(filter: Option<&SessionFilter>) -> (String, Vec<SqlValue>) {
    let mut conds: Vec<String> = Vec::new();
    let mut params: Vec<SqlValue> = Vec::new();
    let Some(f) = filter else {
        return (String::new(), params);
    };
    if let Some(d) = &f.device_scope {
        if !d.is_empty() {
            conds.push("s.device_id = ?".into());
            params.push(SqlValue::Text(d.clone()));
        }
    }
    if let Some(s) = &f.source {
        if !s.is_empty() {
            conds.push("s.source = ?".into());
            params.push(SqlValue::Text(s.clone()));
        }
    }
    if let Some(fav) = f.favorited {
        conds.push(format!("s.favorited = {}", fav as i64));
    }
    if let Some(g) = &f.local_group_id {
        conds.push("s.local_group_id = ?".into());
        params.push(SqlValue::Text(g.clone()));
    }
    if let Some(g) = &f.synced_group_id {
        conds.push("s.synced_group_id = ?".into());
        params.push(SqlValue::Text(g.clone()));
    }
    if let Some(ts) = &f.from_ts {
        if !ts.is_empty() {
            conds.push("s.last_active_at >= ?".into());
            params.push(SqlValue::Text(ts.clone()));
        }
    }
    if let Some(ts) = &f.to_ts {
        if !ts.is_empty() {
            conds.push("s.last_active_at <= ?".into());
            params.push(SqlValue::Text(ts.clone()));
        }
    }
    if let Some(m) = &f.model {
        if !m.is_empty() {
            // EXISTS semantics: the session matched iff ANY usage record in
            // this session used the model. Both keys are required — a session
            // id is a parser file stem, so ids can collide across devices.
            conds.push(
                "EXISTS (SELECT 1 FROM usage_records u \
                 WHERE u.session_id = s.id AND u.device_id = s.device_id AND u.model = ?)"
                    .into(),
            );
            params.push(SqlValue::Text(m.clone()));
        }
    }
    if let Some(q) = &f.search {
        let q = q.trim();
        if !q.is_empty() {
            // Substring search over the DISPLAY title (custom title wins, same
            // COALESCE as the SELECT) and the project path. Like the client-
            // side filter it replaces, the match is case-insensitive and
            // literal — the pattern escapes LIKE wildcards so `%`/`_` in the
            // query never act as metacharacters.
            let pattern = like_pattern(q);
            conds.push(
                "(COALESCE(NULLIF(s.custom_title,''), s.title_orig) LIKE ? ESCAPE '\\' \
                 OR s.project_dir LIKE ? ESCAPE '\\')"
                    .into(),
            );
            params.push(SqlValue::Text(pattern.clone()));
            params.push(SqlValue::Text(pattern));
        }
    }
    let clause = if conds.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conds.join(" AND "))
    };
    (clause, params)
}

/// Wrap a user search query in `%…%` with LIKE metacharacters (`%`, `_`, `\`)
/// escaped — the SQL mirror of the old client-side substring filter, so a
/// literal `%` or `_` in the query matches itself instead of acting as a
/// wildcard. The ESCAPE char is `\` (SQLite's default), quoted with `ESCAPE
/// '\'` in the SQL above.
fn like_pattern(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 2);
    out.push('%');
    for c in q.chars() {
        if c == '%' || c == '_' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// The shared session-list SELECT (rows + live usage aggregate + optional
/// WHERE), ending in a stable time-desc ORDER BY. `device_id`/`id`
/// tiebreakers make the ordering total, so offset paging never duplicates or
/// skips a row across page turns. Callers append `LIMIT ? OFFSET ?` when
/// paging (or leave the clause empty for the full unpaged read).
fn sessions_select_sql(clause: &str) -> String {
    format!(
        "SELECT s.id, s.device_id, s.source, s.project_dir,
                COALESCE(NULLIF(s.custom_title,''), s.title_orig) AS title,
                s.favorited, s.local_group_id, s.synced_group_id,
                s.started_at, s.last_active_at, s.agent_type,
                COALESCE(agg.request_count, 0),
                COALESCE(agg.total_tokens, 0),
                COALESCE(agg.total_cost_usd, 0.0)
         FROM sessions s
         LEFT JOIN (
            SELECT session_id, device_id,
                   COUNT(*) AS request_count,
                   COALESCE(SUM(input_tokens+output_tokens+cache_creation_tokens+cache_read_tokens),0) AS total_tokens,
                   COALESCE(SUM(CAST(total_cost_usd AS REAL)),0) AS total_cost_usd
            FROM usage_records GROUP BY session_id, device_id
         ) agg ON agg.session_id = s.id AND agg.device_id = s.device_id
         {clause}
         ORDER BY s.last_active_at DESC, s.device_id, s.id"
    )
}

/// Decode a `sessions` row in the shared SELECT's column order (13 columns —
/// the positional mapping lives in one place for both the paged and unpaged
/// reads). `project_dir` crosses as the PROJECT IDENTITY
/// ([`crate::model::project_identity`]): a Claude Code worktree suffix
/// (`.claude\worktrees\…`) collapses to its parent project here, at the one
/// decode seam every session-list read goes through — so worktree sessions
/// (subagents et al.) surface under their parent project in every consumer of
/// the list. The stored row keeps the raw launch dir; only the read is
/// truncated.
fn session_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    let project_dir: String = r.get(3)?;
    Ok(SessionRow {
        id: r.get(0)?,
        device_id: r.get(1)?,
        source: r.get(2)?,
        project_dir: project_identity(&project_dir).to_string(),
        title: r.get(4)?,
        favorited: r.get::<_, i64>(5)? != 0,
        local_group_id: r.get(6)?,
        synced_group_id: r.get(7)?,
        started_at: r.get(8)?,
        last_active_at: r.get(9)?,
        agent_type: r.get(10)?,
        request_count: r.get::<_, i64>(11)? as u32,
        total_tokens: r.get::<_, i64>(12)? as u32,
        total_cost_usd: r.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;

    // ---- session_messages (transcript 原文, db source of truth) ----

    /// Every session with a new message is flagged dirty — favorited or not.
    /// The db is the source of truth for ALL sessions; the favorites gate lives
    /// at the derived jsonl snapshot, not here. The messages also round-trip:
    /// all of them landed (db holds every session's 原文).
    #[test]
    fn ingest_session_messages_flags_dirty_for_all_sessions() {
        let s = mem();
        s.ingest_session_messages_marking_dirty(
            "dev1",
            &[
                msg("u1", "s1", SessionMessageRole::User, "2026-07-13T10:00:00Z"),
                msg(
                    "a1",
                    "s1",
                    SessionMessageRole::Assistant,
                    "2026-07-13T10:00:01Z",
                ),
                msg("u2", "s2", SessionMessageRole::User, "2026-07-13T11:00:00Z"),
            ],
        )
        .unwrap();
        assert_eq!(
            s.dirty_sessions().unwrap(),
            vec!["s1".to_string(), "s2".to_string()],
            "both sessions flagged dirty, deduped + sorted"
        );
        let s1 = s.query_session_messages("dev1", "s1").unwrap();
        assert_eq!(s1.len(), 2);
        assert_eq!(s1[0].role, SessionMessageRole::User);
        assert_eq!(s1[1].role, SessionMessageRole::Assistant);
        assert_eq!(s1[1].model.as_deref(), Some("glm-5.2"));
        assert!(s1[0].model.is_none(), "user row stores no model");
    }

    /// Re-ingesting the same (device_id, uuid) writes nothing new, so it must
    /// not re-flag a session dirty — a retried collect must not re-dirty settled
    /// sessions forever. The returned inserted set is the proof.
    #[test]
    fn ingest_session_messages_dedup_is_idempotent() {
        let s = mem();
        let m = msg("u1", "s1", SessionMessageRole::User, "2026-07-13T10:00:00Z");
        let inserted = s
            .ingest_session_messages_marking_dirty("dev1", std::slice::from_ref(&m))
            .unwrap();
        assert_eq!(inserted.len(), 1);
        let re = s
            .ingest_session_messages_marking_dirty("dev1", std::slice::from_ref(&m))
            .unwrap();
        assert!(re.is_empty(), "dedup: re-ingest inserts nothing");
        assert_eq!(s.query_session_messages("dev1", "s1").unwrap().len(), 1);
    }

    /// The same source uuid under two device ids is two rows (device_id is in
    /// the PK) — a session replayed on two devices is kept per device.
    #[test]
    fn ingest_session_messages_keeps_per_device_for_same_uuid() {
        let s = mem();
        let m = msg("u1", "s1", SessionMessageRole::User, "2026-07-13T10:00:00Z");
        s.ingest_session_messages_marking_dirty("dev1", std::slice::from_ref(&m))
            .unwrap();
        let dev2 = s
            .ingest_session_messages_marking_dirty("dev2", std::slice::from_ref(&m))
            .unwrap();
        assert_eq!(dev2.len(), 1, "same uuid under a new device is a new row");
        assert_eq!(s.query_session_messages("dev1", "s1").unwrap().len(), 1);
        assert_eq!(s.query_session_messages("dev2", "s1").unwrap().len(), 1);
    }

    /// role as_str/parse_str round-trips every variant (the db stores the
    /// lowercase spelling; the read path restores the enum).
    #[test]
    fn session_message_role_roundtrips() {
        for role in [
            SessionMessageRole::User,
            SessionMessageRole::Assistant,
            SessionMessageRole::Tool,
            SessionMessageRole::System,
        ] {
            assert_eq!(SessionMessageRole::parse_str(role.as_str()), role);
        }
    }

    /// Cross-device transcript read: every device's rows merge, deduped by uuid
    /// with self winning the conflict, then ordered by (ts, uuid).
    #[test]
    fn query_session_transcript_merges_devices_with_self_priority() {
        let s = mem();
        s.ingest_session_messages_marking_dirty(
            "dev1",
            &[
                msg("u1", "s1", SessionMessageRole::User, "2026-07-13T10:00:00Z"),
                msg(
                    "a1",
                    "s1",
                    SessionMessageRole::Assistant,
                    "2026-07-13T10:00:05Z",
                ),
            ],
        )
        .unwrap();
        // dev2 holds the SAME uuid u1 (would lose to self) plus a dev2-only row.
        s.ingest_session_messages_marking_dirty(
            "dev2",
            &[
                msg(
                    "u1",
                    "s1",
                    SessionMessageRole::System,
                    "2026-07-13T10:00:00Z",
                ),
                msg(
                    "a2",
                    "s1",
                    SessionMessageRole::Assistant,
                    "2026-07-13T10:00:10Z",
                ),
            ],
        )
        .unwrap();
        let t = s.query_session_transcript("s1", "dev1").unwrap();
        assert_eq!(t.len(), 3, "u1 deduped (dev1 wins), a1 + a2 kept");
        let u1 = t.iter().find(|m| m.uuid == "u1").unwrap();
        assert_eq!(
            u1.role,
            SessionMessageRole::User,
            "self wins on uuid conflict"
        );
        // Chronological by (ts, uuid): u1, a1, a2.
        assert_eq!(
            t.iter().map(|m| m.uuid.as_str()).collect::<Vec<_>>(),
            vec!["u1", "a1", "a2"]
        );
    }

    /// Worktree sessions surface under their parent project on the production
    /// read path: a row stored with a `.claude\worktrees\…` project_dir (the
    /// launch dir Claude Code gives subagent/parallel sessions, issue #84)
    /// comes back from `query_sessions_page` truncated to the parent — every
    /// consumer of the session list reasons about the parent project, while
    /// the stored row keeps the raw launch dir (the snapshot meta read below
    /// pins that the raw value is NOT rewritten).
    #[test]
    fn query_sessions_page_collapses_worktree_project_to_parent() {
        let s = mem();
        s.upsert_session(
            "dev",
            &SessionSystemData {
                id: "agent-a10c476b".into(),
                source: "claude_code".into(),
                project_dir: "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b".into(),
                title_orig: "核实 cc-switch 供应商".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-02T00:00:00.000Z".into(),
                agent_type: "Explore".into(),
            },
        )
        .unwrap();
        let rows = s
            .query_sessions_page(&SessionQuery {
                filter: None,
                limit: 50,
                offset: 0,
            })
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_dir, "D:\\Project\\O_CC_One");
        assert_eq!(rows[0].agent_type, "Explore", "subagent tag crosses as-is");
        // The stored row (and thus the git snapshot meta) keeps the RAW launch
        // dir — truncation is a read-side rule, not a rewrite.
        let meta = s
            .get_session_snapshot_meta("dev", "agent-a10c476b")
            .unwrap()
            .unwrap();
        assert_eq!(
            meta.project_dir,
            "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b"
        );
    }

    /// The other policy half: `import_session_snapshot` (pull) uses
    /// `RefreshSystemAndFavorites`, so on conflict it overwrites the
    /// favorites-track columns (a peer is authoritative for its own row's
    /// favorited / synced_group_id) but leaves the device-local columns
    /// (custom_title, local_group_id) untouched.
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
