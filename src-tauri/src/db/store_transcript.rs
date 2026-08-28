//! `session_messages` 表的消息本体：transcript 原文的 ingest 与读取路径。
//!
//! ALL 会话的原文都进这张表（不限收藏）——SQLite 是 message 原文的唯一真相源，
//! 查看路径（`query_session_messages` / `query_session_transcript`）只读 db。
//! ingest 的同事务标脏规则经 `super::store_sessions_writes::mark_sessions_dirty`
//! 落地（其列轨道权威 `PushTrack` 也在那边）；`sessions` 表的读写路径分别在
//! `super::store_sessions_writes` / `super::store_sessions_reads`。

use super::store_sessions_writes::mark_sessions_dirty;
use super::*;

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

#[cfg(test)]
mod tests {
    use crate::db::testutil::*;
    use crate::model::SessionMessageRole;

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
}
