//! Device registry CRUD + self-heal + local forget.

use super::*;

impl super::Store {
    // ---------------- Devices ----------------

    /// Register/refresh a device in the registry.
    pub fn upsert_device(
        &self,
        device_id: &str,
        display_name: &str,
        is_self: bool,
    ) -> AppResult<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        conn.execute(
            "INSERT INTO device (device_id, display_name, is_self, first_seen)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(device_id) DO UPDATE SET
               display_name=excluded.display_name,
               is_self=excluded.is_self",
            params![
                device_id,
                display_name,
                is_self as i64,
                crate::time::now_iso()
            ],
        )?;
        Ok(())
    }

    /// Self-heal the `device` table from `usage_records`: any device that has
    /// usage rows but no `device` row (e.g. a peer that never published its
    /// `config/devices_<id>.json` name artifact) gets a fallback row with a
    /// generated `Device-<prefix>` name. `ON CONFLICT DO NOTHING` preserves
    /// names already learned via `reload_devices_into_store` — this only fills
    /// gaps, never overwrites. `is_self` is left 0 here; the command layer
    /// re-derives it from `cfg.device_id` on read, so a stale stored value can
    /// never mislabel a peer as "this device". `first_seen` takes the device's
    /// earliest usage timestamp (more truthful than `now`).
    pub fn discover_devices_from_usage(&self) -> AppResult<()> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT device_id, MIN(timestamp)
             FROM usage_records
             WHERE device_id NOT IN (SELECT device_id FROM device)
             GROUP BY device_id",
        )?;
        let gaps: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (device_id, first_seen) in gaps {
            let name = crate::config::default_display_name(&device_id);
            conn.execute(
                "INSERT INTO device (device_id, display_name, is_self, first_seen)
                 VALUES (?1, ?2, 0, ?3)
                 ON CONFLICT(device_id) DO NOTHING",
                params![device_id, name, first_seen],
            )?;
        }
        Ok(())
    }

    pub fn list_devices(&self) -> AppResult<Vec<crate::model::DeviceInfo>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT device_id, display_name, is_self, first_seen FROM device ORDER BY is_self DESC, device_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(crate::model::DeviceInfo {
                device_id: r.get(0)?,
                display_name: r.get(1)?,
                is_self: r.get::<_, i64>(2)? != 0,
                first_seen: r.get(3)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// All device_ids currently in the registry. Reconcile uses this to find
    /// rows whose backing git presence has vanished.
    pub fn list_device_ids(&self) -> AppResult<Vec<String>> {
        let conn = self.conn.lock().expect("db mutex poisoned");
        let mut stmt = conn.prepare("SELECT device_id FROM device")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(AppError::from)
    }

    /// Locally forget a device: drop its registry row and ALL its local data
    /// footprint — every device-keyed table (the list is schema-owned:
    /// `schema::DEVICE_KEYED_TABLES`, arity-pinned against the DDL by the
    /// `device_keyed_tables_match_schema` test) plus the one exception below.
    /// No Git effect — a peer still in the repo reappears on the next pull,
    /// which force-checks-out its still-published files back into the worktree
    /// and fully re-imports them (registry entry, per-day usage JSONL, favorite
    /// snapshots — pinned by
    /// `forget_device_local_is_undone_by_a_pull_that_restores_the_snapshot`
    /// in `sync::domains`'s no-git round-trip suite). The caller MUST guard
    /// `is_self` (this device is never forgettable). Returns the total rows
    /// removed.
    pub fn forget_device_local(&self, device_id: &str) -> AppResult<usize> {
        let mut conn = self.conn.lock().expect("db mutex poisoned");
        let tx = conn.transaction()?;
        let mut deleted = 0;
        // The EXCEPTION, first: `dirty_sessions` has no device column — resolve
        // through `sessions`, so this must run BEFORE the mechanical loop below
        // deletes those `sessions` rows (it reads the rows being deleted). Only
        // ids NO OTHER device holds: session ids can collide across devices, and
        // a shared id's dirty flag also guards the survivors' pending push —
        // deleting it would silently drop a surviving device's un-pushed
        // snapshot recompute. This subquery stays hand-written (outside the
        // schema-owned list) because the survivor-aware resolution is logic,
        // not a column purge.
        deleted += tx.execute(
            "DELETE FROM dirty_sessions WHERE session_id IN (\
                SELECT id FROM sessions WHERE device_id = ?1 \
                  AND id NOT IN (SELECT id FROM sessions WHERE device_id != ?1))",
            params![device_id],
        )?;
        // The mechanical half: one DELETE per device-keyed table, driven by the
        // schema-owned list — a new device-keyed table joins this loop the
        // moment its row is added to `schema::DEVICE_KEYED_TABLES` (the schema
        // arity test forces the list to follow the DDL, so forget can never
        // silently miss a table).
        for table in schema::DEVICE_KEYED_TABLES {
            deleted += tx.execute(
                &format!("DELETE FROM {table} WHERE device_id = ?1"),
                params![device_id],
            )?;
        }
        tx.commit()?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::*;

    #[test]
    fn forget_device_local_purges_all_its_data() {
        let s = mem();
        s.upsert_device("aaaaaaaaaaaa", "Device-aaaa", false)
            .unwrap();
        s.upsert_device("bbbbbbbbbbbb", "Device-bbbb", false)
            .unwrap();
        s.ingest(&[
            rec("u1", "2026-07-13", "glm-5.2", "aaaaaaaaaaaa", 100, 50, 0.0),
            rec("u2", "2026-07-13", "glm-5.2", "bbbbbbbbbbbb", 200, 80, 0.0),
        ])
        .unwrap();

        let deleted = s.forget_device_local("aaaaaaaaaaaa").unwrap();
        // device row + usage_records.
        assert!(deleted >= 2, "expected several rows deleted, got {deleted}");

        let ids = s.list_device_ids().unwrap();
        assert!(
            !ids.iter().any(|i| i == "aaaaaaaaaaaa"),
            "forgotten device must be gone from the registry"
        );
        assert!(ids.iter().any(|i| i == "bbbbbbbbbbbb"));

        // Forgotten device's usage is gone; the survivor keeps its row.
        let gone = s
            .count_logs(&UsageFilter {
                device_scope: Some("aaaaaaaaaaaa".into()),
                ..UsageFilter::default()
            })
            .unwrap();
        assert_eq!(gone, 0);
        let kept = s
            .count_logs(&UsageFilter {
                device_scope: Some("bbbbbbbbbbbb".into()),
                ..UsageFilter::default()
            })
            .unwrap();
        assert_eq!(kept, 1);
    }

    /// 遗忘一台设备清掉的足迹是「该设备本地的一切」（架构审查Ⅲ候选③）：
    /// forget 后表驱动走全部读路径——usage、turn、会话列表（生产分页路
    /// 径）、收藏、消息、dirty——均查不到该设备；幸存设备不受影响。共享
    /// session id 的 dirty 旗标留给幸存设备（其待推送重算不因遗忘对端而
    /// 丢——id 可跨设备碰撞）。
    #[test]
    fn forget_device_local_erases_the_footprint_on_every_read_path() {
        let s = mem();
        let peer = "aaaaaaaaaaaa";
        let self_dev = "0123456789ab";
        s.upsert_device(self_dev, "Self", true).unwrap();
        s.upsert_device(peer, "Peer", false).unwrap();

        // peer 的完整足迹：收藏会话 + 消息（dirty）+ usage + turn。
        seed_session_project(&s, "p-sess", peer, "/proj/peer", "2026-08-10T10:00:00.000Z");
        s.set_session_favorited(peer, "p-sess", true).unwrap();
        s.ingest_session_messages_marking_dirty(
            peer,
            &[msg(
                "m1",
                "p-sess",
                SessionMessageRole::User,
                "2026-08-10T10:00:00Z",
            )],
        )
        .unwrap();
        let mut usage = rec("u1", "2026-07-13", "glm-5.2", peer, 100, 50, 1.0);
        usage.session_id = "p-sess".into();
        s.ingest(&[usage]).unwrap();
        s.ingest_turn_durations(&[TurnDuration {
            uuid: "t1".into(),
            timestamp: "2026-07-13T10:00:00Z".into(),
            day: "2026-07-13".into(),
            session_id: "p-sess".into(),
            device_id: peer.into(),
            duration_ms: 90_000,
        }])
        .unwrap();
        // 共享 session id：self 也采到同 id 会话且 dirty。
        seed_session_project(
            &s,
            "shared",
            self_dev,
            "/proj/mine",
            "2026-08-11T10:00:00.000Z",
        );
        s.ingest_session_messages_marking_dirty(
            self_dev,
            &[msg(
                "m-s",
                "shared",
                SessionMessageRole::User,
                "2026-08-11T10:00:00Z",
            )],
        )
        .unwrap();
        seed_session_project(&s, "shared", peer, "/proj/peer", "2026-08-12T10:00:00.000Z");
        s.ingest_session_messages_marking_dirty(
            peer,
            &[msg(
                "m-p",
                "shared",
                SessionMessageRole::User,
                "2026-08-12T10:00:00Z",
            )],
        )
        .unwrap();
        let peer_scope = UsageFilter {
            device_scope: Some(peer.into()),
            ..UsageFilter::default()
        };
        assert_eq!(s.count_logs(&peer_scope).unwrap(), 1, "前提：peer 有用量");

        s.forget_device_local(peer).unwrap();

        // 表驱动：每条读路径对 peer 一无所见。
        let session_filter = SessionFilter {
            device_scope: Some(peer.into()),
            ..Default::default()
        };
        let checks: [(&str, bool); 6] = [
            (
                "usage（count_logs）",
                s.count_logs(&peer_scope).unwrap() == 0,
            ),
            (
                "turn（query_stats.turn_count）",
                s.query_stats(&peer_scope).unwrap().turn_count == 0,
            ),
            (
                "会话列表（query_sessions_page）",
                s.query_sessions_page(&SessionQuery {
                    filter: Some(session_filter.clone()),
                    limit: 50,
                    offset: 0,
                })
                .unwrap()
                .is_empty(),
            ),
            (
                "收藏（favorited_session_ids）",
                s.favorited_session_ids(peer).unwrap().is_empty(),
            ),
            (
                "消息（query_session_messages）",
                s.query_session_messages(peer, "p-sess").unwrap().is_empty(),
            ),
            (
                "registry（list_device_ids）",
                !s.list_device_ids().unwrap().iter().any(|i| i == peer),
            ),
        ];
        for (path, invisible) in checks {
            assert!(invisible, "forget 后读路径仍可见：{path}");
        }
        // dirty：peer 专属会话的旗标随行消失；共享 id 的旗标留给 self。
        let dirty = s.dirty_sessions().unwrap();
        assert!(!dirty.contains(&"p-sess".to_string()));
        assert!(
            dirty.contains(&"shared".to_string()),
            "幸存设备的待推送重算不因遗忘对端而丢"
        );
        // 幸存设备足迹完好。
        assert!(s.get_session("shared", self_dev).unwrap().is_some());
        assert_eq!(
            s.query_session_messages(self_dev, "shared").unwrap().len(),
            1
        );
    }
}
