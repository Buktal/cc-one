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

    /// One session row by its exact composite key `(id, device_id)` — the
    /// usage-side "request log → session" jump channel: the frontend resolves a
    /// usage record's `session_id` into the session row (title + identity) via
    /// this read instead of a backend join on the usage query. Same SELECT as
    /// the list (usage aggregates + project_identity truncation included), so
    /// the resolved row is identical to what the session list would show.
    /// `None` = no such session (usage record without a collected session).
    pub fn get_session(&self, id: &str, device_id: &str) -> AppResult<Option<SessionRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let sql = sessions_select_sql("WHERE s.id = ?1 AND s.device_id = ?2");
        conn.query_row(&sql, params![id, device_id], session_row)
            .optional()
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

    /// The project dimension: sessions rolled up by project identity, joined
    /// live with their `usage_records` aggregates (requests / token
    /// four-buckets / cost — the usage table stays the single source of token
    /// truth, nothing is stored at project grain). One bucket per
    /// `project_identity(s.project_dir)` value — the SQL scalar backed by the
    /// one Rust rule — so Claude Code worktree sessions and their usage land
    /// under the PARENT project. `MAX(last_active_at)` feeds the
    /// recent-activity metric and orders the buckets (most recent first, `pid`
    /// tiebreaker for determinism). Sessions with NO usage still form their
    /// bucket's session count (LEFT JOIN + COALESCE): the dimension describes
    /// where sessions ran, not only where usage landed. The filter applies
    /// BEFORE grouping, so a time range narrows which sessions feed the
    /// buckets at all.
    ///
    /// One SYNTHETIC row carries the [`UNKNOWN_PROJECT`] sentinel: the
    /// aggregate over session-less usage — remote usage whose favorite
    /// snapshot was never pulled (the only cross-device session rows are
    /// favorites), plus session-less legacy rows. Without it, that usage
    /// silently vanished from every project view. `session_count` is 0 by
    /// definition (no session rows exist); `last_active_at` is the MAX usage
    /// timestamp so the bucket sorts by real recency. Session-attribute
    /// constraints the bucket can never satisfy (favorited-only, a group, a
    /// search) suppress the row entirely; `favorited = Some(false)` keeps it
    /// (session-less is definitionally not favorited).
    pub fn query_project_stats(
        &self,
        filter: Option<&SessionFilter>,
    ) -> AppResult<Vec<ProjectStatsRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let sql = format!(
            "SELECT project_identity(s.project_dir) AS pid,
                    COUNT(*) AS session_count,
                    COALESCE(SUM(agg.request_count), 0) AS request_count,
                    COALESCE(SUM(agg.input_tokens), 0) AS input_tokens,
                    COALESCE(SUM(agg.output_tokens), 0) AS output_tokens,
                    COALESCE(SUM(agg.cache_creation_tokens), 0) AS cache_creation_tokens,
                    COALESCE(SUM(agg.cache_read_tokens), 0) AS cache_read_tokens,
                    COALESCE(SUM(agg.total_cost_usd), 0.0) AS total_cost_usd,
                    MAX(s.last_active_at) AS last_active_at
             FROM sessions s
             LEFT JOIN (
                SELECT session_id, device_id,
                       COUNT(*) AS request_count,
                       SUM(input_tokens) AS input_tokens,
                       SUM(output_tokens) AS output_tokens,
                       SUM(cache_creation_tokens) AS cache_creation_tokens,
                       SUM(cache_read_tokens) AS cache_read_tokens,
                       SUM(CAST(total_cost_usd AS REAL)) AS total_cost_usd
                FROM usage_records GROUP BY session_id, device_id
             ) agg ON agg.session_id = s.id AND agg.device_id = s.device_id
             {clause}
             GROUP BY pid
             ORDER BY last_active_at DESC, pid"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            let tokens = TokenCounts {
                input: r.get::<_, i64>(3)? as u32,
                output: r.get::<_, i64>(4)? as u32,
                cache_creation: r.get::<_, i64>(5)? as u32,
                cache_read: r.get::<_, i64>(6)? as u32,
            };
            Ok(ProjectStatsRow {
                project_dir: r.get(0)?,
                session_count: r.get::<_, i64>(1)? as u32,
                request_count: r.get::<_, i64>(2)? as u32,
                // Both derived metrics reuse TokenCounts' single
                // implementations — the same ones the dashboard's stats and
                // per-model rows use (output is not in the hit-rate
                // denominator; the formula ignores it).
                total_tokens: tokens.total(),
                input_tokens: tokens.input,
                output_tokens: tokens.output,
                cache_creation_tokens: tokens.cache_creation,
                cache_read_tokens: tokens.cache_read,
                cache_hit_rate: tokens.cache_hit_rate(),
                total_cost_usd: r.get(7)?,
                last_active_at: r.get(8)?,
            })
        })?;
        let mut out = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;

        // ---- the synthetic unknown row (see method doc) ----
        // Session-attribute constraints session-less usage can never satisfy
        // suppress the bucket. A group filter matches via the session's group
        // column and search via the session's title/project/message bodies —
        // all absent for session-less rows.
        let suppress_unknown = filter.is_some_and(|f| {
            f.favorited == Some(true)
                || f.local_group_id.is_some()
                || f.synced_group_id.is_some()
                || f.search.as_deref().is_some_and(|s| !s.trim().is_empty())
        });
        if !suppress_unknown {
            // Apply the filter's OTHER dimensions at usage grain: time runs on
            // `u.timestamp` (no session row exists to read `last_active_at`
            // from); device / source / model map to their usage columns. Note
            // the model asymmetry: known buckets gate "session USED the model"
            // then sum its FULL usage, while the unknown bucket can only match
            // per-row (`u.model = ?`) — it has no session to gate on.
            let mut conds: Vec<String> = vec!["NOT EXISTS (SELECT 1 FROM sessions s \
                  WHERE s.id = u.session_id AND s.device_id = u.device_id)"
                .into()];
            let mut uparams: Vec<SqlValue> = Vec::new();
            if let Some(f) = filter {
                if let Some(d) = &f.device_scope {
                    if !d.is_empty() {
                        conds.push("u.device_id = ?".into());
                        uparams.push(SqlValue::Text(d.clone()));
                    }
                }
                if let Some(s) = &f.source {
                    if !s.is_empty() {
                        conds.push("u.source = ?".into());
                        uparams.push(SqlValue::Text(s.clone()));
                    }
                }
                if let Some(m) = &f.model {
                    if !m.is_empty() {
                        conds.push("u.model = ?".into());
                        uparams.push(SqlValue::Text(m.clone()));
                    }
                }
                if let Some(ts) = &f.from_ts {
                    if !ts.is_empty() {
                        conds.push("u.timestamp >= ?".into());
                        uparams.push(SqlValue::Text(ts.clone()));
                    }
                }
                if let Some(ts) = &f.to_ts {
                    if !ts.is_empty() {
                        conds.push("u.timestamp <= ?".into());
                        uparams.push(SqlValue::Text(ts.clone()));
                    }
                }
            }
            let usql = format!(
                "SELECT COUNT(*),
                        COALESCE(SUM(input_tokens),0),
                        COALESCE(SUM(output_tokens),0),
                        COALESCE(SUM(cache_creation_tokens),0),
                        COALESCE(SUM(cache_read_tokens),0),
                        COALESCE(SUM(CAST(total_cost_usd AS REAL)),0),
                        COALESCE(MAX(timestamp),'')
                 FROM usage_records u WHERE {}",
                conds.join(" AND ")
            );
            let (request_count, input, output, cc, cr, cost, last): (
                i64,
                i64,
                i64,
                i64,
                i64,
                f64,
                String,
            ) = conn.query_row(&usql, params_from_iter(uparams.iter()), |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?;
            if request_count > 0 {
                let tokens = TokenCounts {
                    input: input as u32,
                    output: output as u32,
                    cache_creation: cc as u32,
                    cache_read: cr as u32,
                };
                out.push(ProjectStatsRow {
                    project_dir: UNKNOWN_PROJECT.to_string(),
                    session_count: 0,
                    request_count: request_count as u32,
                    total_tokens: tokens.total(),
                    input_tokens: tokens.input,
                    output_tokens: tokens.output,
                    cache_creation_tokens: tokens.cache_creation,
                    cache_read_tokens: tokens.cache_read,
                    cache_hit_rate: tokens.cache_hit_rate(),
                    total_cost_usd: cost,
                    last_active_at: last,
                });
                // Keep the bucket ordering contract (recency desc, key asc)
                // over the appended row too.
                out.sort_by(|a, b| {
                    b.last_active_at
                        .cmp(&a.last_active_at)
                        .then_with(|| a.project_dir.cmp(&b.project_dir))
                });
            }
        }
        Ok(out)
    }

    /// The stats dimension at SESSION grain: every session (unpaged, list
    /// order) with its usage four-buckets / hit rate / cost, its
    /// `session_messages` row count, and its per-model token split. The
    /// sessions workbench consumes this one read for everything the paged
    /// list cannot answer — the left tree's node aggregates, the right rail's
    /// per-session and per-project cards, and the duration buckets. Same
    /// sources and rules as `query_project_stats` (live `usage_records`
    /// aggregates via a LEFT JOIN so usage-less sessions still appear;
    /// `project_identity` truncation at the decode seam) — only the grain
    /// differs, so the two dimensions can never disagree on a session's
    /// numbers. The SQL emits one row per (session, model); the fold below
    /// merges them into one `SessionStatsRow` per session.
    pub fn query_session_stats(
        &self,
        filter: Option<&SessionFilter>,
    ) -> AppResult<Vec<SessionStatsRow>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let (clause, params_vec) = build_session_where(filter);
        let sql = format!(
            "SELECT s.id, s.device_id, s.source, s.project_dir,
                    COALESCE(NULLIF(s.custom_title,''), s.title_orig) AS title,
                    s.favorited, s.local_group_id, s.synced_group_id,
                    s.started_at, s.last_active_at, s.agent_type,
                    COALESCE(u.request_count, 0),
                    COALESCE(m.message_count, 0),
                    COALESCE(u.input_tokens, 0),
                    COALESCE(u.output_tokens, 0),
                    COALESCE(u.cache_creation_tokens, 0),
                    COALESCE(u.cache_read_tokens, 0),
                    COALESCE(u.total_cost_usd, 0.0),
                    u.model
             FROM sessions s
             LEFT JOIN (
                SELECT session_id, device_id, model,
                       COUNT(*) AS request_count,
                       SUM(input_tokens) AS input_tokens,
                       SUM(output_tokens) AS output_tokens,
                       SUM(cache_creation_tokens) AS cache_creation_tokens,
                       SUM(cache_read_tokens) AS cache_read_tokens,
                       SUM(CAST(total_cost_usd AS REAL)) AS total_cost_usd
                FROM usage_records GROUP BY session_id, device_id, model
             ) u ON u.session_id = s.id AND u.device_id = s.device_id
             LEFT JOIN (
                SELECT session_id, device_id, COUNT(*) AS message_count
                FROM session_messages GROUP BY session_id, device_id
             ) m ON m.session_id = s.id AND m.device_id = s.device_id
             {clause}
             ORDER BY s.last_active_at DESC, s.device_id, s.id, u.model"
        );
        let mut stmt = conn.prepare(&sql)?;
        let raw = stmt.query_map(params_from_iter(params_vec.iter()), |r| {
            // Columns 13-16 are COALESCE'd to 0 for the usage-less LEFT JOIN
            // row, so a plain read works; only u.model (18) is nullable.
            let input = r.get::<_, i64>(13)?;
            let output = r.get::<_, i64>(14)?;
            let cache_creation = r.get::<_, i64>(15)?;
            let cache_read = r.get::<_, i64>(16)?;
            let model: Option<String> = r.get(18)?;
            let project_dir: String = r.get(3)?;
            let row = SessionStatsRow {
                id: r.get(0)?,
                device_id: r.get(1)?,
                source: r.get(2)?,
                // Same decode-seam truncation as `session_row`: the identity
                // the list shows, so the tree buckets built on these rows
                // match the project aggregate.
                project_dir: project_identity(&project_dir).to_string(),
                title: r.get(4)?,
                favorited: r.get::<_, i64>(5)? != 0,
                local_group_id: r.get(6)?,
                synced_group_id: r.get(7)?,
                started_at: r.get(8)?,
                last_active_at: r.get(9)?,
                agent_type: r.get(10)?,
                request_count: r.get::<_, i64>(11)? as u32,
                message_count: r.get::<_, i64>(12)? as u32,
                input_tokens: input as u32,
                output_tokens: output as u32,
                cache_creation_tokens: cache_creation as u32,
                cache_read_tokens: cache_read as u32,
                cache_hit_rate: 0.0,
                total_cost_usd: r.get(17)?,
                models: Vec::new(),
            };
            let slice = SessionModelTokens {
                model: model.unwrap_or_default(),
                tokens: (input + output + cache_creation + cache_read) as u32,
            };
            Ok((row, slice))
        })?;
        let mut per_session: Vec<(SessionStatsRow, SessionModelTokens)> = raw
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)?;

        // Fold consecutive (session, model) rows into one row per session —
        // the ORDER BY keeps a session's model rows adjacent, and the fold's
        // key check makes an ordering regression surface as duplicate rows
        // instead of silently merged sessions. Bucket sums, request count and
        // cost accumulate across the model rows; message_count is identical
        // on every row of the session (the join is session-grain).
        let mut out: Vec<SessionStatsRow> = Vec::with_capacity(per_session.len());
        for (row, slice) in per_session.drain(..) {
            match out.last_mut() {
                Some(prev) if prev.id == row.id && prev.device_id == row.device_id => {
                    prev.request_count += row.request_count;
                    prev.message_count = row.message_count;
                    prev.input_tokens += row.input_tokens;
                    prev.output_tokens += row.output_tokens;
                    prev.cache_creation_tokens += row.cache_creation_tokens;
                    prev.cache_read_tokens += row.cache_read_tokens;
                    prev.total_cost_usd += row.total_cost_usd;
                    prev.models.push(slice);
                }
                _ => {
                    let mut row = row;
                    row.models.push(slice);
                    out.push(row);
                }
            }
        }
        for row in &mut out {
            let tokens = TokenCounts {
                input: row.input_tokens,
                output: row.output_tokens,
                cache_creation: row.cache_creation_tokens,
                cache_read: row.cache_read_tokens,
            };
            row.cache_hit_rate = tokens.cache_hit_rate();
            // Drop the usage-less phantom slice (empty model, zero tokens) so
            // a session without usage renders "no model data", not a blank row.
            row.models
                .retain(|m| !(m.model.is_empty() && m.tokens == 0));
            row.models.sort_by_key(|m| std::cmp::Reverse(m.tokens));
        }
        Ok(out)
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
/// `("", [])`. `pub(super)` so the distinct-projects read (store_reads) reuses
/// the same sessions-side narrowing — one builder, no drifting copy.
pub(super) fn build_session_where(filter: Option<&SessionFilter>) -> (String, Vec<SqlValue>) {
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
    if let Some(p) = &f.project {
        if !p.is_empty() {
            // Match by project IDENTITY via the `project_identity` SQL scalar
            // (the one Rust rule, registered as a UDF) — a worktree session's
            // raw launch dir collapses to its parent, so it matches the parent
            // project's filter. Same function the project aggregate groups by,
            // so filtering and bucketing can never disagree. The unknown
            // sentinel matches the EMPTY identity — the sessions-side face of
            // the unknown bucket (a session row exists but carries no launch
            // dir; the usage-side NOT EXISTS face lives in store_reads).
            if p == UNKNOWN_PROJECT {
                conds.push("project_identity(s.project_dir) = ''".into());
            } else {
                conds.push("project_identity(s.project_dir) = ?".into());
                params.push(SqlValue::Text(p.clone()));
            }
        }
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
            // COALESCE as the SELECT), the project path, and every message BODY
            // (`session_messages.content`). Like the client-side filter it
            // replaces, the match is case-insensitive and literal — the pattern
            // escapes LIKE wildcards so `%`/`_` in the query never act as
            // metacharacters. The body probe is an EXISTS at session-id grain,
            // deliberately NOT scoped to `s.device_id`: the transcript a row
            // opens is `query_session_transcript`, which merges ALL devices'
            // messages for the id (deduped by uuid, self winning) — so the
            // search must see the same union, or a hit could open a transcript
            // that doesn't contain it (a peer's pulled snapshot often holds
            // messages self's local file lacks, and vice versa). The uuid-level
            // self-wins collapse only ever drops same-uuid duplicates (the same
            // source event), so the union is the merged transcript for matching
            // purposes. `idx_session_messages_sid` serves the probe.
            let pattern = like_pattern(q);
            conds.push(
                "(COALESCE(NULLIF(s.custom_title,''), s.title_orig) LIKE ? ESCAPE '\\' \
                 OR s.project_dir LIKE ? ESCAPE '\\' \
                 OR EXISTS (SELECT 1 FROM session_messages m \
                            WHERE m.session_id = s.id \
                            AND m.content LIKE ? ESCAPE '\\'))"
                    .into(),
            );
            params.push(SqlValue::Text(pattern.clone()));
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

    /// The jump read resolves by the FULL composite key: the same session id
    /// can exist under two devices (a session collected on both), and usage
    /// aggregates must come from that device's records only. A key that
    /// matches no row resolves to `None` (session-less historical usage).
    #[test]
    fn get_session_resolves_by_composite_key_with_usage_aggregate() {
        let s = mem();
        for (dev, title) in [("dev-a", "本机采集的会话"), ("dev-b", "peer 同 id 会话")] {
            s.upsert_session(
                dev,
                &SessionSystemData {
                    id: "sid-1".into(),
                    source: "claude_code".into(),
                    project_dir: "D:\\Project\\O_CC_One".into(),
                    title_orig: title.into(),
                    started_at: "2026-08-01T00:00:00.000Z".into(),
                    last_active_at: "2026-08-02T00:00:00.000Z".into(),
                    agent_type: String::new(),
                },
            )
            .unwrap();
        }
        // bound_rec 侧 helper 固定 device "dev"，这里需要 dev-a —— 就地组记录。
        let mut r = rec("u1", "2026-08-15", "glm-5.2", "dev-a", 100, 50, 0.25);
        r.session_id = "sid-1".into();
        s.ingest_marking_dirty(&[r]).unwrap();
        let a = s.get_session("sid-1", "dev-a").unwrap().unwrap();
        assert_eq!(a.device_id, "dev-a");
        assert_eq!(a.title, "本机采集的会话");
        assert_eq!(a.request_count, 1, "usage aggregate joins on device too");
        assert_eq!(a.total_tokens, 150);
        assert_eq!(a.total_cost_usd, 0.25);
        let b = s.get_session("sid-1", "dev-b").unwrap().unwrap();
        assert_eq!(b.title, "peer 同 id 会话");
        assert_eq!(b.request_count, 0, "peer row has no usage of its own");
        assert!(s.get_session("sid-1", "dev-x").unwrap().is_none());
        assert!(s.get_session("sid-2", "dev-a").unwrap().is_none());
    }

    /// Seed one usage record bound to `sid` with explicit token buckets +
    /// cost (the project aggregate's inputs).
    fn bound_rec(s: &Store, uuid: &str, sid: &str, tokens: TokenCounts, cost: f64) {
        let mut r = rec(
            uuid,
            "2026-08-15",
            "glm-5.2",
            "dev",
            tokens.input,
            tokens.output,
            cost,
        );
        r.session_id = sid.into();
        r.tokens = tokens;
        s.ingest_marking_dirty(&[r]).unwrap();
    }

    /// The project dimension's core rollup: per project — session count,
    /// request count, token four-buckets, cost, and MAX(last_active_at) — with
    /// sessions that have NO usage still counting toward their bucket's
    /// session count (LEFT JOIN + COALESCE), and buckets ordered most-recent
    /// first. Cache-hit rate reuses `TokenCounts::cache_hit_rate` (the same
    /// single formula the dashboard and per-model rows read).
    #[test]
    fn project_stats_rolls_up_sessions_usage_and_recency_per_project() {
        let s = mem();
        seed_session_project(&s, "a1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "a2", "dev", "/proj/alpha", "2026-08-12T10:00:00.000Z");
        seed_session_project(&s, "b1", "dev", "/proj/beta", "2026-08-11T10:00:00.000Z");
        bound_rec(
            &s,
            "u1",
            "a1",
            TokenCounts {
                input: 100,
                output: 20,
                cache_read: 70,
                cache_creation: 0,
            },
            1.0,
        );
        bound_rec(
            &s,
            "u2",
            "a2",
            TokenCounts {
                input: 50,
                output: 0,
                cache_read: 50,
                cache_creation: 0,
            },
            2.0,
        );

        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(rows.len(), 2, "one bucket per project identity");
        // Most recent first: alpha (08-12) before beta (08-11).
        assert_eq!(rows[0].project_dir, "/proj/alpha");
        assert_eq!(rows[0].session_count, 2);
        assert_eq!(rows[0].request_count, 2);
        assert_eq!(rows[0].input_tokens, 150);
        assert_eq!(rows[0].output_tokens, 20);
        assert_eq!(rows[0].cache_read_tokens, 120);
        assert_eq!(rows[0].total_tokens, 290);
        assert!((rows[0].total_cost_usd - 3.0).abs() < 1e-9);
        // cache_read / (input + cache_creation + cache_read) = 120 / 270
        // (the cacheable pool includes cache_read itself).
        assert!((rows[0].cache_hit_rate - 120.0 / 270.0).abs() < 1e-9);
        assert_eq!(rows[0].last_active_at, "2026-08-12T10:00:00.000Z");

        // Beta has one session but zero usage: the bucket survives with
        // zeroed aggregates (the dimension describes where sessions ran).
        assert_eq!(rows[1].project_dir, "/proj/beta");
        assert_eq!(rows[1].session_count, 1);
        assert_eq!(rows[1].request_count, 0);
        assert_eq!(rows[1].total_tokens, 0);
        assert_eq!(rows[1].cache_hit_rate, 0.0);
        assert_eq!(rows[1].total_cost_usd, 0.0);
    }

    /// Worktree sessions aggregate under their PARENT project (issue #84's
    /// rule, applied by the `project_identity` SQL scalar at GROUP BY): the
    /// parent bucket absorbs the worktree session, its usage, and its newer
    /// last_active_at — while an unrelated project stays its own bucket.
    #[test]
    fn project_stats_collapses_worktree_sessions_into_parent() {
        let s = mem();
        seed_session_project(
            &s,
            "s-main",
            "dev",
            "D:\\Project\\O_CC_One",
            "2026-08-02T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-agent",
            "dev",
            "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b",
            "2026-08-03T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-other",
            "dev",
            "D:\\Project\\Other",
            "2026-08-04T10:00:00.000Z",
        );
        bound_rec(
            &s,
            "u1",
            "s-main",
            TokenCounts {
                input: 10,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            0.5,
        );
        bound_rec(
            &s,
            "u2",
            "s-agent",
            TokenCounts {
                input: 20,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            1.5,
        );

        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.project_dir.as_str())
                .collect::<Vec<_>>(),
            ["D:\\Project\\Other", "D:\\Project\\O_CC_One"],
            "two buckets: the worktree never forms its own"
        );
        let parent = rows.last().unwrap();
        assert_eq!(parent.session_count, 2, "main + worktree session");
        assert_eq!(parent.request_count, 2, "both sessions' usage landed");
        assert!((parent.total_cost_usd - 2.0).abs() < 1e-9);
        assert_eq!(
            parent.last_active_at, "2026-08-03T10:00:00.000Z",
            "MAX over the bucket incl. the worktree session"
        );
    }

    /// The SessionFilter project dimension: matching runs through
    /// `project_identity`, so filtering to the parent project returns BOTH the
    /// parent's own sessions and its worktree sessions — in the paged list,
    /// the sidebar counts, and the project aggregate alike. A project nobody
    /// ran in matches nothing.
    #[test]
    fn session_filter_project_matches_worktree_sessions_to_parent() {
        let s = mem();
        seed_session_project(
            &s,
            "s-main",
            "dev",
            "/proj/alpha",
            "2026-08-02T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-agent",
            "dev",
            "/proj/alpha/.claude/worktrees/agent-x",
            "2026-08-03T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "s-other",
            "dev",
            "/proj/beta",
            "2026-08-04T10:00:00.000Z",
        );

        let f = SessionFilter {
            project: Some("/proj/alpha".into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions_page(&SessionQuery {
                filter: Some(f.clone()),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["s-agent", "s-main"], "worktree matches the parent");

        let counts = s.count_sessions(Some(&f), "local").unwrap();
        assert_eq!(counts.total, 2);

        let buckets = s.query_project_stats(Some(&f)).unwrap();
        assert_eq!(
            buckets.len(),
            1,
            "the filter narrows the aggregate's buckets"
        );
        assert_eq!(buckets[0].project_dir, "/proj/alpha");
        assert_eq!(buckets[0].session_count, 2);

        // A project with no sessions matches nothing anywhere.
        let none = SessionFilter {
            project: Some("/proj/gone".into()),
            ..Default::default()
        };
        assert!(s
            .query_sessions_page(&SessionQuery {
                filter: Some(none.clone()),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .is_empty());
        assert_eq!(s.count_sessions(Some(&none), "local").unwrap().total, 0);
        assert!(s.query_project_stats(Some(&none)).unwrap().is_empty());
    }

    /// The unknown-project bucket (#100): usage with NO session row forms one
    /// synthetic [`UNKNOWN_PROJECT`] row instead of vanishing. Local legacy
    /// rows (empty session_id) and unresolvable session ids land there alike;
    /// a known project's bucket is untouched; `session_count` is 0 by
    /// definition and `last_active_at` is the MAX usage timestamp, so the
    /// bucket sorts by real recency.
    #[test]
    fn project_stats_appends_unknown_bucket_for_session_less_usage() {
        let s = mem();
        seed_session_project(&s, "a1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        bound_rec(
            &s,
            "u1",
            "a1",
            TokenCounts {
                input: 100,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            1.0,
        );
        // Two flavors of session-less usage: a legacy row (empty session_id)
        // and a row whose session id resolves to no sessions row (the remote
        // shape — a peer's session that was never favorited, so no snapshot
        // was pulled).
        let mut legacy = rec("u2", "2026-08-15", "glm-5.2", "dev", 10, 0, 0.5);
        legacy.session_id = String::new();
        let mut remote = rec("u3", "2026-08-16", "glm-5.2", "peer", 20, 10, 1.5);
        remote.session_id = "never-pulled".into();
        s.ingest(&[legacy, remote]).unwrap();

        let rows = s.query_project_stats(None).unwrap();
        assert_eq!(rows.len(), 2, "alpha bucket + the synthetic unknown row");
        let unknown = rows
            .iter()
            .find(|r| r.project_dir == UNKNOWN_PROJECT)
            .expect("synthetic unknown row present");
        assert_eq!(
            unknown.session_count, 0,
            "no session rows exist by definition"
        );
        assert_eq!(unknown.request_count, 2);
        assert_eq!(unknown.input_tokens, 30);
        assert_eq!(unknown.output_tokens, 10);
        assert_eq!(unknown.total_tokens, 40);
        assert_eq!(unknown.total_cost_usd, 2.0);
        // cache_read / (input + cache_creation + cache_read) = 0 / 30 = 0.
        assert_eq!(unknown.cache_hit_rate, 0.0);
        assert_eq!(unknown.last_active_at, "2026-08-16T10:00:00.000Z");
        // Recency ordering: the unknown bucket (08-16) sorts before alpha
        // (last_active 08-10).
        assert_eq!(rows[0].project_dir, UNKNOWN_PROJECT);

        let alpha = rows
            .iter()
            .find(|r| r.project_dir == "/proj/alpha")
            .unwrap();
        assert_eq!(alpha.request_count, 1, "alpha's bucket untouched");

        // A favorited-only filter can never be satisfied by session-less
        // usage — the unknown row is suppressed, known buckets remain.
        let fav = SessionFilter {
            favorited: Some(true),
            ..Default::default()
        };
        let rows = s.query_project_stats(Some(&fav)).unwrap();
        assert!(
            rows.iter().all(|r| r.project_dir != UNKNOWN_PROJECT),
            "favorited-only suppresses the unknown bucket"
        );

        // A time window that excludes the session-less rows drops the bucket
        // entirely (filter applies before aggregation).
        let early = SessionFilter {
            to_ts: Some("2026-08-15T00:00:00.000Z".into()),
            ..Default::default()
        };
        let rows = s.query_project_stats(Some(&early)).unwrap();
        assert!(
            rows.iter().all(|r| r.project_dir != UNKNOWN_PROJECT),
            "window without session-less usage yields no unknown row"
        );
    }

    /// The cross-device shape the bucket exists for (#94): a peer's FAVORITED
    /// session arrives as a pulled snapshot → its usage lands under the
    /// snapshot's project; the same peer's NON-favorited session has no
    /// snapshot → its usage lands in the unknown bucket, not nowhere.
    #[test]
    fn project_stats_unknown_bucket_covers_remote_nonfavorited_usage() {
        let s = mem();
        let peer = "peerdev01";
        // Pulled favorite snapshot: session row for the peer, project /remote.
        s.import_session_snapshot(
            peer,
            &SessionSnapshotMeta {
                v: SESSION_SNAPSHOT_VERSION,
                id: "fav-1".into(),
                source: "claude_code".into(),
                project_dir: "/remote".into(),
                title_orig: "Favorited".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-12T10:00:00.000Z".into(),
                agent_type: String::new(),
                favorited: true,
                synced_group_id: String::new(),
            },
            &[],
        )
        .unwrap();
        let mut fav_usage = rec("ru1", "2026-08-15", "glm-5.2", peer, 100, 0, 1.0);
        fav_usage.session_id = "fav-1".into();
        let mut plain_usage = rec("ru2", "2026-08-15", "glm-5.2", peer, 40, 0, 4.0);
        plain_usage.session_id = "plain-1".into();
        s.ingest(&[fav_usage, plain_usage]).unwrap();

        let rows = s.query_project_stats(None).unwrap();
        let remote = rows.iter().find(|r| r.project_dir == "/remote").unwrap();
        assert_eq!(
            remote.request_count, 1,
            "favorited snapshot's usage bucketed"
        );
        let unknown = rows
            .iter()
            .find(|r| r.project_dir == UNKNOWN_PROJECT)
            .expect("non-favorited remote usage did not vanish");
        assert_eq!(unknown.request_count, 1);
        assert_eq!(unknown.input_tokens, 40);
    }

    /// The sessions-side unknown sentinel: it matches sessions whose project
    /// identity is EMPTY (a session row exists but carries no launch dir) —
    /// the sessions face of the unknown bucket, mirroring the usage-side NOT
    /// EXISTS face. Worktree/sessioned projects never match it.
    #[test]
    fn session_filter_unknown_sentinel_matches_project_less_sessions() {
        let s = mem();
        seed_session_project(&s, "s1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "s2", "dev", "", "2026-08-11T10:00:00.000Z");

        let f = SessionFilter {
            project: Some(UNKNOWN_PROJECT.into()),
            ..Default::default()
        };
        let ids: Vec<String> = s
            .query_sessions_page(&SessionQuery {
                filter: Some(f.clone()),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(ids, ["s2"], "only the project-less session matches");

        // The paged list, the sidebar counts, and the project aggregate all
        // share the same clause — the aggregate narrows to the "" bucket.
        assert_eq!(s.count_sessions(Some(&f), "local").unwrap().total, 1);
        let buckets = s.query_project_stats(Some(&f)).unwrap();
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].project_dir, "");
    }

    /// The stats dimension at session grain: one row per session (folded from
    /// its per-(session, model) SQL rows) carrying the session's identity, its
    /// usage four-buckets / hit rate / cost, its message count, and its
    /// per-model token split most-tokens-first. A session with NO usage still
    /// appears with zeroed aggregates and no phantom model slice.
    #[test]
    fn session_stats_folds_model_rows_per_session() {
        let s = mem();
        seed_session_project(&s, "a1", "dev", "/proj/alpha", "2026-08-10T10:00:00.000Z");
        seed_session_project(&s, "b1", "dev", "/proj/beta", "2026-08-11T10:00:00.000Z");
        // Two usage records on a1 across two models: the fold must sum the
        // buckets/cost/requests AND keep one model slice per model.
        bound_rec(
            &s,
            "u1",
            "a1",
            TokenCounts {
                input: 100,
                output: 20,
                cache_read: 70,
                cache_creation: 10,
            },
            1.0,
        );
        let mut r = rec("u2", "2026-08-15", "glm-5.2-air", "dev", 30, 0, 2.0);
        r.session_id = "a1".into();
        r.tokens = TokenCounts {
            input: 30,
            output: 0,
            cache_read: 60,
            cache_creation: 0,
        };
        s.ingest_marking_dirty(&[r]).unwrap();
        // Two transcript messages for a1 — the message count follows
        // session_messages, not usage_records.
        s.ingest_session_messages_marking_dirty(
            "dev",
            &[
                msg("m1", "a1", SessionMessageRole::User, "2026-07-13T10:00:00Z"),
                msg(
                    "m2",
                    "a1",
                    SessionMessageRole::Assistant,
                    "2026-07-13T10:00:01Z",
                ),
            ],
        )
        .unwrap();

        let rows = s.query_session_stats(None).unwrap();
        assert_eq!(rows.len(), 2, "one row per session, never per model");
        // List order: most recent first (b1 08-11 before a1 08-10).
        assert_eq!(rows[0].id, "b1");
        assert_eq!(rows[0].request_count, 0);
        assert_eq!(rows[0].message_count, 0);
        assert_eq!(rows[0].total_cost_usd, 0.0);
        assert_eq!(rows[0].cache_hit_rate, 0.0);
        assert!(
            rows[0].models.is_empty(),
            "usage-less session renders no phantom model slice"
        );

        let a1 = &rows[1];
        assert_eq!(a1.id, "a1");
        assert_eq!(a1.request_count, 2);
        assert_eq!(a1.message_count, 2);
        assert_eq!(a1.input_tokens, 130);
        assert_eq!(a1.output_tokens, 20);
        assert_eq!(a1.cache_creation_tokens, 10);
        assert_eq!(a1.cache_read_tokens, 130);
        assert!((a1.total_cost_usd - 3.0).abs() < 1e-9);
        // Same single formula the project grain reads:
        // cache_read / (input + cache_creation + cache_read) = 130/270.
        assert!((a1.cache_hit_rate - 130.0 / 270.0).abs() < 1e-9);
        assert_eq!(
            a1.models
                .iter()
                .map(|m| (m.model.as_str(), m.tokens))
                .collect::<Vec<_>>(),
            [("glm-5.2", 200), ("glm-5.2-air", 90)],
            "per-model slices, most-tokens-first, bucket sums intact"
        );
    }

    /// The session grain applies the same project-identity truncation as the
    /// list decode seam and groups nothing — but its `project_dir` output must
    /// match what the list shows, so a worktree session stats row carries the
    /// PARENT project (the tree buckets the frontend builds on top stay
    /// consistent with the project aggregate).
    #[test]
    fn session_stats_collapses_worktree_project_to_parent() {
        let s = mem();
        seed_session_project(
            &s,
            "s-agent",
            "dev",
            "D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b",
            "2026-08-03T10:00:00.000Z",
        );
        let rows = s.query_session_stats(None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project_dir, "D:\\Project\\O_CC_One");
    }

    /// The filter applies BEFORE grouping: a time range narrows which sessions
    /// feed the buckets, so a session excluded by the window drops out of both
    /// the session count and its usage out of the token/cost aggregates.
    #[test]
    fn project_stats_time_filter_narrows_buckets_before_grouping() {
        let s = mem();
        seed_session_project(
            &s,
            "a-old",
            "dev",
            "/proj/alpha",
            "2026-08-01T10:00:00.000Z",
        );
        seed_session_project(
            &s,
            "a-new",
            "dev",
            "/proj/alpha",
            "2026-08-10T10:00:00.000Z",
        );
        seed_session_project(&s, "b-mid", "dev", "/proj/beta", "2026-08-05T10:00:00.000Z");
        bound_rec(
            &s,
            "u1",
            "a-old",
            TokenCounts {
                input: 100,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            1.0,
        );
        bound_rec(
            &s,
            "u2",
            "a-new",
            TokenCounts {
                input: 10,
                output: 0,
                cache_read: 0,
                cache_creation: 0,
            },
            0.5,
        );

        let from = SessionFilter {
            from_ts: Some("2026-08-04T00:00:00.000Z".into()),
            ..Default::default()
        };
        let rows = s.query_project_stats(Some(&from)).unwrap();
        assert_eq!(rows.len(), 2, "alpha keeps its newer session");
        let alpha = rows
            .iter()
            .find(|r| r.project_dir == "/proj/alpha")
            .unwrap();
        assert_eq!(alpha.session_count, 1, "a-old excluded by the window");
        assert_eq!(alpha.request_count, 1, "its usage dropped with it");
        assert!((alpha.total_cost_usd - 0.5).abs() < 1e-9);
    }

    /// Cross-session full-text search: the `search` filter also matches message
    /// BODIES, on the production paths (paged list + sidebar counts share
    /// `build_session_where`). Case-insensitive and literal — a `%` in the query
    /// matches a literal `%` in a body, never acts as a wildcard.
    #[test]
    fn session_filter_search_matches_message_bodies() {
        let s = mem();
        seed_session(&s, "s1", "dev", "2026-08-01T10:00:00.000Z");
        seed_session(&s, "s2", "dev", "2026-08-02T10:00:00.000Z");
        let mut hit = msg("u1", "s1", SessionMessageRole::User, "2026-08-01T10:00:00Z");
        hit.content = "the tokamak calibration notes".into();
        let mut pct = msg("u2", "s2", SessionMessageRole::User, "2026-08-02T10:00:00Z");
        pct.content = "shipment 100% done".into();
        s.ingest_session_messages_marking_dirty("dev", &[hit, pct])
            .unwrap();

        let ids = |q: &str| -> Vec<String> {
            s.query_sessions_page(&SessionQuery {
                filter: Some(SessionFilter {
                    search: Some(q.into()),
                    ..Default::default()
                }),
                limit: 50,
                offset: 0,
            })
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
        };
        assert_eq!(
            ids("tokamak"),
            ["s1"],
            "body-only hit surfaces the session (title/project miss)"
        );
        assert_eq!(ids("TOKAMAK"), ["s1"], "body match is case-insensitive");
        assert_eq!(
            ids("00%"),
            ["s2"],
            "literal % in a body match, not a wildcard"
        );
        assert!(ids("glorb").is_empty(), "no body/title/project match");
        // Sidebar counts go through the same clause — they must agree with the
        // paged list, or the paginator would contradict the rows it counts.
        let counts = s
            .count_sessions(
                Some(&SessionFilter {
                    search: Some("tokamak".into()),
                    ..Default::default()
                }),
                "local",
            )
            .unwrap();
        assert_eq!(counts.total, 1, "counts see the body hit too");
    }

    /// The body probe reuses the transcript MERGE semantics: a message that
    /// exists only under a PEER's device id (a pulled snapshot row) still
    /// matches the session, because the transcript the row opens
    /// (`query_session_transcript`) merges all devices' messages for the id.
    /// A device-scoped probe would miss it and show a hit-less list while the
    /// opened transcript contains the match. Pinned end-to-end here: the
    /// Local-tab shape (device_scope = self) matches, and the merged transcript
    /// actually holds the peer-only message.
    #[test]
    fn session_filter_search_sees_peer_device_message_bodies() {
        let s = mem();
        // Self collected the session; its own slice does NOT contain the term.
        seed_session(&s, "s1", "dev", "2026-08-01T10:00:00.000Z");
        let mut own = msg("u1", "s1", SessionMessageRole::User, "2026-08-01T10:00:00Z");
        own.content = "own-device chatter".into();
        s.ingest_session_messages_marking_dirty("dev", &[own])
            .unwrap();
        // A peer's favorited snapshot carries an extra message self never saw —
        // imported through the production pull path, under the PEER's device id.
        let mut extra = msg(
            "p1",
            "s1",
            SessionMessageRole::Assistant,
            "2026-08-01T11:00:00Z",
        );
        extra.content = "the zeppelin docking checklist".into();
        s.import_session_snapshot(
            "peer1",
            &SessionSnapshotMeta {
                v: SESSION_SNAPSHOT_VERSION,
                id: "s1".into(),
                source: "claude_code".into(),
                project_dir: "/proj".into(),
                title_orig: "Title".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-01T12:00:00.000Z".into(),
                agent_type: String::new(),
                favorited: true,
                synced_group_id: String::new(),
            },
            &[extra],
        )
        .unwrap();

        let filter = SessionFilter {
            device_scope: Some("dev".into()),
            search: Some("zeppelin".into()),
            ..Default::default()
        };
        let rows = s
            .query_sessions_page(&SessionQuery {
                filter: Some(filter),
                limit: 50,
                offset: 0,
            })
            .unwrap();
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["s1"],
            "self's row matches via the peer-only body (Local-tab shape)"
        );
        // The consistency premise this pins: opening the session really does
        // show the peer-only message in the merged transcript.
        let merged = s.query_session_transcript("s1", "dev").unwrap();
        assert!(
            merged.iter().any(|m| m.content.contains("zeppelin")),
            "merged transcript holds the peer message the search matched"
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
