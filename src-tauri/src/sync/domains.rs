//! Per-syncable-domain pairs — the single home for "what each domain syncs".
//!
//! Every domain that rides the git repo declares BOTH halves of its pair here:
//! the push-side materialize (store → derived files) and the pull-side import
//! (peer files → store). `flow` only composes these pairs with git primitives
//! (order, retry, commit/push) — it no longer knows a domain's file shapes.
//! Adding a new syncable domain = one module section here (its materialize +
//! import) plus one line in each of `flow::push_usage` / `flow::pull_and_import`;
//! the store never grows a new concern just to sync one.
//!
//! The domains:
//!   - usage: per-day `usage-<day>.jsonl` / `turns-<day>.jsonl` Artifacts,
//!     driven by the `dirty_days` flag (collect marks, push clears).
//!   - sessions: per-session `sessions/<id>.jsonl` snapshots, driven by the
//!     `dirty_sessions` flag; only favorited sessions have a snapshot
//!     ("snapshot exists ⇔ favorited", `snapshot_policy`).
//!   - providers: per-device key-stripped `providers.json`; byte-stable, so no
//!     dirty flag is needed.
//!   - devices: per-device name artifacts (`config/devices_<id>.json`), written
//!     on the config side; this domain only imports them into the registry.
//!
//! The dirty-flag clear is shared by the usage + sessions domains: ONE
//! transaction (`Store::clear_dirty_flags_if_unchanged`) so days and sessions
//! clear together or not at all — a mid-clear failure can never leave
//! days-clean + sessions-dirty.

use std::collections::{BTreeMap, BTreeSet};

use crate::config::{ConfigData, Paths};
use crate::db::{DaySnapshot, SessionCounts, Store};
use crate::error::AppResult;
use crate::sessions::snapshot_policy::{
    decide_snapshot_action, presence_mismatches, SnapshotAction,
};

// ---------------------------------------------------------------------------
// usage — per-day usage + turns Artifacts
// ---------------------------------------------------------------------------

/// Push-side materialize: recompute every dirty day's `usage-<day>.jsonl` and
/// `turns-<day>.jsonl` from the store (uuid-ordered, byte-stable). Returns the
/// recompute-time row-count snapshots — the clear phase re-checks these before
/// dropping a day's dirty flag, so a row that raced in after the recompute
/// keeps the day dirty.
pub fn usage_materialize(
    store: &Store,
    paths: &Paths,
    device_id: &str,
) -> AppResult<Vec<DaySnapshot>> {
    let dirty = store.dirty_days()?;
    let mut day_snapshots: Vec<DaySnapshot> = Vec::with_capacity(dirty.len());
    for day in &dirty {
        let usage = crate::collect::artifact::recompute_usage_day(store, paths, device_id, day)?;
        let turns = crate::collect::artifact::recompute_turns_day(store, paths, device_id, day)?;
        day_snapshots.push(DaySnapshot {
            day: day.clone(),
            usage_rows: usage,
            turn_rows: turns,
        });
    }
    Ok(day_snapshots)
}

/// Pull-side import: read every device's usage + turns Artifacts into the
/// store, deduped by the `(uuid, device_id)` primary key. Imported rows are
/// already on git, so their days are NOT marked dirty (the pull/collect split
/// `Store::ingest` vs `Store::ingest_marking_dirty` encodes). Returns the
/// number of newly inserted usage records.
pub fn usage_import(store: &Store, paths: &Paths) -> AppResult<u32> {
    let records = crate::collect::artifact::read_all_artifacts(paths)?;
    let inserted = store.ingest(&records)?;
    let turns = crate::collect::artifact::read_all_turn_artifacts(paths)?;
    store.ingest_turn_durations(&turns)?;
    Ok(inserted.len() as u32)
}

// ---------------------------------------------------------------------------
// sessions — per-session snapshots
// ---------------------------------------------------------------------------

/// What the sessions materialize wrote, for the shared clear phase: the
/// favorited sessions whose snapshot was recomputed (cleared only when their
/// message count is unchanged) and the non-favorited sessions whose leftover
/// snapshot was deleted (cleared unconditionally — deletion is idempotent).
pub struct SessionsMaterialized {
    pub recomputed: Vec<SessionCounts>,
    pub removed: Vec<String>,
}

/// Push-side materialize: for every dirty session, write the derived jsonl
/// snapshot when favorited, DELETE any leftover snapshot when not (the local
/// half of un-favorite propagation — a peer pulling sees the file vanish).
/// The favorites gate lives HERE, not in collect: the db is the source of
/// truth for all messages, but only favorited sessions get a snapshot file.
pub fn sessions_materialize(
    store: &Store,
    paths: &Paths,
    device_id: &str,
) -> AppResult<SessionsMaterialized> {
    let dirty_sessions = store.dirty_sessions()?;
    let mut recomputed: Vec<SessionCounts> = Vec::with_capacity(dirty_sessions.len());
    let mut removed: Vec<String> = Vec::new();
    for sid in &dirty_sessions {
        let favorited = store
            .get_session_favorited(device_id, sid)?
            .unwrap_or(false);
        match decide_snapshot_action(favorited) {
            // favorited ⇒ the snapshot must exist: recompute it from the store.
            SnapshotAction::Write => {
                let count = crate::sessions::session_snapshot::recompute_session_snapshot(
                    store, paths, device_id, sid,
                )?;
                recomputed.push(SessionCounts {
                    session_id: sid.clone(),
                    message_rows: count,
                });
            }
            // not favorited ⇒ the snapshot must not exist. Idempotent: a
            // never-favorited session has no file to remove.
            SnapshotAction::Remove => {
                let path = paths.session_snapshot_path(device_id, sid);
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
                removed.push(sid.clone());
            }
        }
    }
    Ok(SessionsMaterialized {
        recomputed,
        removed,
    })
}

/// Pull-side import: import peers' session snapshots into the store and
/// propagate cross-device un-favorites. Self's own snapshots are skipped on
/// read ([`crate::sessions::session_snapshot::read_all_session_snapshots`]), so
/// self's rows are never overwritten by a possibly-stale git copy of itself.
/// For every peer that has (or had) a favorited session row, sessions whose
/// snapshot file vanished since the last pull are un-favorited and their shared
/// messages dropped — the pull-side counterpart to the push-side jsonl
/// deletion.
pub fn sessions_import(store: &Store, paths: &Paths, self_device_id: &str) -> AppResult<()> {
    let snapshots =
        crate::sessions::session_snapshot::read_all_session_snapshots(paths, self_device_id)?;
    // still-favorited ids per peer = the snapshot files that exist this pull.
    let mut per_device: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for snap in &snapshots {
        per_device
            .entry(snap.device_id.clone())
            .or_default()
            .insert(snap.meta.id.clone());
        store.import_session_snapshot(&snap.device_id, &snap.meta, &snap.messages)?;
    }
    // Reconcile every peer with a favorited row — including ones that shipped
    // no files this pull (they may have un-favorited everything). The sessions
    // to un-favorite here = the peer's favorited sessions whose snapshot file
    // vanished, computed by the shared snapshot_policy oracle so push and pull
    // agree on what "in sync" means (the push path enforces the same invariant
    // for this device via `decide_snapshot_action`).
    for peer in store.favorited_session_devices(self_device_id)? {
        let still_present = per_device.remove(&peer).unwrap_or_default();
        let peer_favorited: BTreeSet<String> =
            store.favorited_session_ids(&peer)?.into_iter().collect();
        let to_unfavorite =
            presence_mismatches(&still_present, &peer_favorited).favorites_without_files;
        store.bulk_unfavorite_sessions(&peer, &to_unfavorite)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// providers — per-device key-stripped providers.json
// ---------------------------------------------------------------------------

/// Push-side materialize: write THIS device's `providers.json` from the store,
/// key-stripped (API keys stay in the local DB — the file carries only
/// structure). No dirty flag: the write is byte-stable, so an unchanged store
/// rewrites identical bytes and the commit+push below no-ops.
pub fn providers_materialize(store: &Store, paths: &Paths, device_id: &str) -> AppResult<()> {
    crate::provider::sync::write_own_providers(store, paths, device_id)
}

/// Pull-side import: read peers' key-stripped provider structure into the
/// store, latest-wins with local keys merged back (an import never overwrites
/// a local key).
pub fn providers_import(store: &Store, paths: &Paths, self_device_id: &str) -> AppResult<()> {
    crate::provider::sync::import_peer_providers(store, paths, self_device_id)
}

// ---------------------------------------------------------------------------
// devices — per-device name artifacts (registry)
// ---------------------------------------------------------------------------

/// Pull-side import: reload the (just-pulled) cloud device registry into the
/// store and reconcile dirty devices (a device with no git presence is pruned).
/// This domain has no push-side materialize — the name artifact is written on
/// the config side (`devices::ensure_own_device_artifact`), not by the push
/// flow; it rides the normal commit+push.
pub fn devices_import(store: &Store, paths: &Paths, cfg: &ConfigData) -> AppResult<()> {
    crate::devices::reload_devices_into_store(store, paths, cfg)
}

// ---------------------------------------------------------------------------
// No-git round-trip tests: materialize (store → files) then import (files →
// fresh store) must recover the same content, per domain, without a real repo.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testutil::{mem, msg, rec};
    use crate::model::{App, Provider, ProviderCategory, SessionMessageRole, TurnDuration};

    fn tmp_paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        (tmp, paths)
    }

    /// The usage domain round-trips without git: collect marks days dirty and
    /// writes only the store; `usage_materialize` recomputes both Artifact
    /// grains; a fresh store's `usage_import` recovers the same rows (usage +
    /// turns), NOT re-marked dirty.
    #[test]
    fn usage_materialize_then_import_roundtrips_without_git() {
        let (_tmp, paths) = tmp_paths();
        let dev = "aabbccddeeff";
        let store_a = mem();
        store_a
            .ingest_marking_dirty(&[
                rec("u1", "2026-07-13", "glm-5.2", dev, 100, 50, 1.0),
                rec("u2", "2026-07-14", "gpt-4o", dev, 10, 0, 0.0),
            ])
            .unwrap();
        store_a
            .ingest_turn_durations_marking_dirty(&[TurnDuration {
                uuid: "t1".into(),
                timestamp: "2026-07-13T10:00:00Z".into(),
                day: "2026-07-13".into(),
                device_id: dev.into(),
                duration_ms: 100_000,
            }])
            .unwrap();

        // Materialize: both dirty days recomputed into per-day files.
        let snapshots = usage_materialize(&store_a, &paths, dev).unwrap();
        assert_eq!(snapshots.len(), 2, "one snapshot per dirty day");
        assert!(paths
            .device_data_dir(dev)
            .join("usage-2026-07-13.jsonl")
            .exists());
        assert!(paths
            .device_data_dir(dev)
            .join("turns-2026-07-13.jsonl")
            .exists());

        // Import into a fresh store: same rows back, days NOT re-dirtied.
        let store_b = mem();
        let n = usage_import(&store_b, &paths).unwrap();
        assert_eq!(n, 2, "both usage records imported");
        assert_eq!(
            store_b.usage_for_day_device("2026-07-13", dev).unwrap(),
            store_a.usage_for_day_device("2026-07-13", dev).unwrap(),
            "usage rows round-trip through the Artifact"
        );
        let turns_b = store_b.turns_for_day_device("2026-07-13", dev).unwrap();
        assert_eq!(turns_b.len(), 1);
        assert_eq!(turns_b[0].uuid, "t1");
        assert!(
            store_b.dirty_days().unwrap().is_empty(),
            "imported rows are already on git — never marked dirty"
        );
    }

    /// The sessions domain round-trips without git: a favorited session's
    /// snapshot materializes from the store, a fresh store's import recovers
    /// meta + favorited + messages, and self's own snapshot is never imported
    /// back (self is local-authoritative).
    #[test]
    fn sessions_materialize_then_import_roundtrips_without_git() {
        use crate::model::SessionSystemData;

        let (_tmp, paths) = tmp_paths();
        let dev = "aabbccddeeff";
        let store_a = mem();
        crate::collect::ingest::ingest_sessions(
            &store_a,
            dev,
            &[SessionSystemData {
                id: "sx".into(),
                source: "claude_code".into(),
                project_dir: "/p".into(),
                title_orig: "Title".into(),
                started_at: "2026-08-01T00:00:00.000Z".into(),
                last_active_at: "2026-08-02T00:00:00.000Z".into(),
                agent_type: "Explore".into(),
            }],
            &[msg(
                "u1",
                "sx",
                SessionMessageRole::User,
                "2026-08-01T10:00:00Z",
            )],
        )
        .unwrap();
        store_a.set_session_favorited(dev, "sx", true).unwrap();

        let m = sessions_materialize(&store_a, &paths, dev).unwrap();
        assert_eq!(m.recomputed.len(), 1);
        assert!(paths.session_snapshot_path(dev, "sx").exists());
        assert!(m.removed.is_empty());

        // A different device pulls: meta + favorited + message all arrive.
        let store_b = mem();
        sessions_import(&store_b, &paths, "001122334455").unwrap();
        let row = store_b
            .query_sessions(None)
            .unwrap()
            .into_iter()
            .find(|r| r.id == "sx")
            .unwrap();
        assert_eq!(row.device_id, dev, "snapshot attributed to its author");
        assert_eq!(row.title, "Title");
        assert_eq!(row.agent_type, "Explore");
        assert!(row.favorited, "favorited rode the meta line");
        assert_eq!(store_b.query_session_messages(dev, "sx").unwrap().len(), 1);

        // Self's own snapshot is skipped on import — the git copy of itself
        // must never overwrite fresher local state.
        let store_c = mem();
        sessions_import(&store_c, &paths, dev).unwrap();
        assert!(
            store_c.query_sessions(None).unwrap().is_empty(),
            "self's snapshot is never pulled back in"
        );
    }

    /// The providers domain round-trips without git: materialize writes the
    /// key-stripped file; import recovers the structure with the key absent
    /// (keys live only in the local DB).
    #[test]
    fn providers_materialize_then_import_roundtrips_without_git() {
        let (_tmp, paths) = tmp_paths();
        let dev = "aabbccddeeff";
        let store_a = mem();
        let saved = store_a
            .save_provider(Provider {
                id: "abcdef01".into(),
                name: "Kimi".into(),
                website_url: "https://platform.kimi.com".into(),
                category: ProviderCategory::Custom,
                app: App::Claude,
                icon: String::new(),
                icon_color: String::new(),
                sort_index: 0,
                notes: String::new(),
                settings_config: r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.com","ANTHROPIC_AUTH_TOKEN":"sk-a-secret"}}"#
                    .into(),
                meta: r#"{}"#.into(),
                updated_at: "2026-08-01T00:00:00.000Z".into(),
            })
            .unwrap();
        providers_materialize(&store_a, &paths, dev).unwrap();
        let file = std::fs::read_to_string(paths.providers_json_path(dev)).unwrap();
        assert!(!file.contains("sk-a-secret"), "key never enters the file");

        let store_b = mem();
        providers_import(&store_b, &paths, "001122334455").unwrap();
        let row = store_b
            .get_provider(App::Claude, &saved.id)
            .unwrap()
            .expect("peer's structure lands");
        assert_eq!(row.name, "Kimi");
        let cfg: serde_json::Value = serde_json::from_str(&row.settings_config).unwrap();
        assert_eq!(cfg["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.com");
        assert!(
            cfg["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(),
            "key-stripped across the round-trip"
        );
    }

    /// The devices domain imports without git: a peer's published name
    /// artifact lands in the fresh store's registry (self's row comes from
    /// register_self on the config side, not from this import).
    #[test]
    fn devices_import_loads_name_artifacts_without_git() {
        let (_tmp, paths) = tmp_paths();
        let self_id = "0123456789ab";
        let peer = "aabbccddeeff";
        std::fs::create_dir_all(&paths.repo_config).unwrap();
        crate::devices::ensure_own_device_artifact(&paths, peer, "Peer One").unwrap();

        let store = mem();
        let cfg = ConfigData {
            device_id: self_id.into(),
            ..Default::default()
        };
        devices_import(&store, &paths, &cfg).unwrap();
        let ids = store.list_device_ids().unwrap();
        assert!(ids.iter().any(|i| i == peer), "peer registry row landed");
        assert_eq!(ids.len(), 1, "only the published artifact lands");
    }

    /// The un-favorite half of the sessions pair: a favorited session whose
    /// snapshot file vanishes materializes as a REMOVE (no file left behind),
    /// and the pull side un-favorites it on the next import (mirrored without
    /// git via `presence_mismatches` + `bulk_unfavorite_sessions` — the same
    /// store calls `sessions_import` makes).
    #[test]
    fn sessions_unfavorite_removes_snapshot_and_propagates_without_git() {
        use crate::model::SessionSystemData;

        let (_tmp, paths) = tmp_paths();
        let dev = "aabbccddeeff";
        let store_a = mem();
        for sid in ["s1", "s2"] {
            crate::collect::ingest::ingest_sessions(
                &store_a,
                dev,
                &[SessionSystemData {
                    id: sid.into(),
                    source: "claude_code".into(),
                    project_dir: "/p".into(),
                    title_orig: sid.into(),
                    started_at: "2026-08-01T00:00:00.000Z".into(),
                    last_active_at: "2026-08-02T00:00:00.000Z".into(),
                    agent_type: String::new(),
                }],
                &[msg(
                    &format!("u-{sid}"),
                    sid,
                    SessionMessageRole::User,
                    "2026-08-01T10:00:00Z",
                )],
            )
            .unwrap();
            store_a.set_session_favorited(dev, sid, true).unwrap();
        }
        let m = sessions_materialize(&store_a, &paths, dev).unwrap();
        assert_eq!(m.recomputed.len(), 2, "both snapshots written");
        assert!(paths.session_snapshot_path(dev, "s1").exists());

        // s1 un-favorited ⇒ its snapshot file vanishes on the next materialize.
        store_a.set_session_favorited(dev, "s1", false).unwrap();
        let m = sessions_materialize(&store_a, &paths, dev).unwrap();
        assert_eq!(m.removed, vec!["s1".to_string()]);
        assert!(!paths.session_snapshot_path(dev, "s1").exists());
        assert!(paths.session_snapshot_path(dev, "s2").exists());

        // The pull side propagates the vanish: the peer's still-favorited rows
        // minus the files present this pull are exactly the un-favorites.
        let peer = "bbccddee0011";
        let store_b = mem();
        store_b
            .upsert_session(
                peer,
                &SessionSystemData {
                    id: "s1".into(),
                    source: "claude_code".into(),
                    project_dir: "/p".into(),
                    title_orig: "s1".into(),
                    started_at: "2026-08-01T00:00:00.000Z".into(),
                    last_active_at: "2026-08-02T00:00:00.000Z".into(),
                    agent_type: String::new(),
                },
            )
            .unwrap();
        store_b.set_session_favorited(peer, "s1", true).unwrap();
        store_b
            .ingest_session_messages_marking_dirty(
                peer,
                &[msg(
                    "u-p",
                    "s1",
                    SessionMessageRole::User,
                    "2026-08-01T10:00:00Z",
                )],
            )
            .unwrap();
        // Only s2's file exists this pull ⇒ s1 must be un-favorited.
        let to_unfavorite = {
            let still_present: BTreeSet<String> = ["s2".to_string()].into_iter().collect();
            let peer_favorited: BTreeSet<String> = store_b
                .favorited_session_ids(peer)
                .unwrap()
                .into_iter()
                .collect();
            presence_mismatches(&still_present, &peer_favorited).favorites_without_files
        };
        assert_eq!(to_unfavorite, vec!["s1".to_string()]);
        store_b
            .bulk_unfavorite_sessions(peer, &to_unfavorite)
            .unwrap();
        assert!(
            store_b.favorited_session_ids(peer).unwrap().is_empty(),
            "peer's vanished snapshot propagated an un-favorite"
        );
    }
}
