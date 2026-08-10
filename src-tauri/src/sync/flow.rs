//! High-level sync flow: compose the low-level git primitives (`super::git`)
//! with the Local Store (dirty-day / dirty-session tracking) and snapshot_policy
//! to form the full pull → import → commit → push pipeline. This is the only
//! layer that knows about both git AND the store — `super::git` stays pure git,
//! and the store stays pure SQL. Best-effort wrappers (`*_best_effort`) swallow
//! errors for background/exit paths; their fallible counterparts propagate.

use super::git::{
    author_email, commit_all, has_changes, is_ahead_of_origin, open_or_clone, pull, push,
    rebase_and_push, require_synced, PullOutcome,
};
use crate::config::{ConfigData, Paths};
use crate::db::{DaySnapshot, SessionCounts, Store};
use crate::error::AppResult;
use crate::sessions::snapshot_policy::{
    decide_snapshot_action, presence_mismatches, SnapshotAction,
};

// ---------------------------------------------------------------------------
// High-level sync flow: pull → import JSONL → commit → push
// ---------------------------------------------------------------------------

/// Pull the remote and import every device's JSONL Artifact into the Local
/// Store (deduped by the store's `(uuid, device_id)` primary key). Synced-only.
///
/// A fast-forward force-checkout (or a diverge rebase's hard reset) may rewrite
/// this device's own `data/<deviceId>/` files — fine: collect writes the store,
/// not the Artifact, so unpushed rows live in SQLite (flagged in `dirty_days`)
/// and the next push recomputes this device's files from the store. The old
/// snapshot/restore mechanism existed only to protect collect appends.
pub fn pull_and_import(store: &Store, paths: &Paths, cfg: &ConfigData) -> AppResult<u32> {
    let (url, token) = require_synced(cfg)?;
    let repo = open_or_clone(&url, &paths.repo, &token)?;
    // Two-step sync: pull (fetch + fast-forward), then — only on diverge —
    // rebase local-only commits onto the remote tip and push.
    match pull(&repo, &token)? {
        PullOutcome::Diverged(upstream) => {
            rebase_and_push(
                &repo,
                &upstream,
                &token,
                &cfg.display_name,
                &author_email(cfg),
            )?;
        }
        PullOutcome::UpToDate | PullOutcome::FastForwarded => {}
    }
    let records = crate::collect::artifact::read_all_artifacts(paths)?;
    let inserted = store.ingest(&records)?;
    // Per-turn durations (separate grain, uuid-deduped).
    let turns = crate::collect::artifact::read_all_turn_artifacts(paths)?;
    store.ingest_turn_durations(&turns)?;
    // Sessions: import peers' snapshots (self is local-authoritative, skipped
    // on read) and propagate cross-device un-favorites.
    import_peer_sessions(store, paths, &cfg.device_id)?;
    // Providers: import peers' key-stripped structure (self skipped on read;
    // local keys are merged back — an import never overwrites a local key).
    crate::provider::sync::import_peer_providers(store, paths, &cfg.device_id)?;
    // Device-name registry: pull may have added/updated config/devices/*.json.
    crate::devices::reload_devices_into_store(store, paths, cfg)?;
    Ok(inserted.len() as u32)
}

/// Import peers' session snapshots into the store and propagate cross-device
/// un-favorites. Self's own snapshots are skipped on read
/// ([`crate::sessions::session_snapshot::read_all_session_snapshots`]), so self's rows are never
/// overwritten by a possibly-stale git copy of itself. For every peer that has
/// (or had) a favorited session row, sessions whose snapshot file vanished since
/// the last pull are un-favorited and their shared messages dropped — the
/// pull-side counterpart to the push-side jsonl deletion.
fn import_peer_sessions(store: &Store, paths: &Paths, self_device_id: &str) -> AppResult<()> {
    let snapshots =
        crate::sessions::session_snapshot::read_all_session_snapshots(paths, self_device_id)?;
    // still-favorited ids per peer = the snapshot files that exist this pull.
    let mut per_device: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
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
        let peer_favorited: std::collections::BTreeSet<String> =
            store.favorited_session_ids(&peer)?.into_iter().collect();
        let to_unfavorite =
            presence_mismatches(&still_present, &peer_favorited).favorites_without_files;
        store.bulk_unfavorite_sessions(&peer, &to_unfavorite)?;
    }
    Ok(())
}

/// Commit any local Artifact/config change and push it (push). A clean worktree
/// AND no commits ahead of origin is a no-op (returns `false`). `message` is
/// the commit body — pass the semantic of the change so the log reads
/// "cc-one: sync" vs "cc-one: library sync". Errors propagate; for
/// daemon/exit paths that must not bubble, use [`commit_and_push_best_effort`].
/// Synced-only.
///
/// The "ahead of origin" half matters for retry: if a previous push failed
/// after its commit landed, the worktree is clean but the local branch is
/// ahead — skipping the push there would strand the commit until an unrelated
/// change re-dirtied the worktree.
pub fn commit_and_push(paths: &Paths, cfg: &ConfigData, message: &str) -> AppResult<bool> {
    let (url, token) = require_synced(cfg)?;
    let repo = open_or_clone(&url, &paths.repo, &token)?;
    let changed = has_changes(&repo)?;
    let ahead = is_ahead_of_origin(&repo)?;
    if !changed && !ahead {
        return Ok(false);
    }
    if changed {
        let email = author_email(cfg);
        commit_all(&repo, message, &cfg.display_name, &email)?;
    }
    push(&repo, &token)?;
    Ok(true)
}

/// Best-effort commit + push for background/exit paths. Standalone is a no-op;
/// a push failure is logged, never propagated — the next collect/sync round
/// carries the change up. The one caller that needs the error surfaced (manual
/// 「立即同步」) calls [`commit_and_push`] directly.
pub fn commit_and_push_best_effort(paths: &Paths, cfg: &ConfigData, message: &str) {
    if !cfg.is_synced() {
        return;
    }
    if let Err(e) = commit_and_push(paths, cfg, message) {
        eprintln!("[cc-one] push failed: {e}");
    }
}

/// Sync push: materialize this device's un-pushed days, session snapshots AND
/// provider structure (key-stripped `providers.json`) from the store, then
/// commit + push, clearing the dirty flags only once the push lands. This is
/// the push-side counterpart to collect's store-only
/// writes: collect flags days/sessions dirty; this recomputes each dirty day's
/// per-day Artifact (`recompute_usage_day` / `recompute_turns_day`) and each
/// dirty session's jsonl snapshot (`recompute_session_snapshot`), commits the
/// rewritten files, pushes, and on success clears the flags (a failed push
/// leaves them dirty for the next retry). Synced-only; a no-op (`false`) when
/// there is nothing dirty to recompute and nothing else to push.
///
/// The session favorites gate lives HERE (not in collect): a favorited dirty
/// session gets its snapshot rewritten; a non-favorited dirty session gets any
/// leftover `sessions/<id>.jsonl` removed — the local half of un-favorite
/// propagation (a peer pulling sees the file vanish). The clear is scoped to
/// recompute-time row/message counts so a raced new row/message keeps its
/// day/session dirty (see [`crate::db::Store::clear_dirty_days_if_unchanged`] /
/// [`crate::db::Store::clear_dirty_sessions_if_unchanged`]).
///
/// Library sync does NOT call this — it has no store/dirty concern and uses
/// [`commit_and_push`] directly.
pub fn push_usage(store: &Store, paths: &Paths, cfg: &ConfigData) -> AppResult<bool> {
    let dirty = store.dirty_days()?;
    // Recompute-time row counts — the clear boundary: rows that land AFTER
    // these snapshots must keep their day dirty.
    let mut day_snapshots: Vec<DaySnapshot> = Vec::with_capacity(dirty.len());
    for day in &dirty {
        let usage =
            crate::collect::artifact::recompute_usage_day(store, paths, &cfg.device_id, day)?;
        let turns =
            crate::collect::artifact::recompute_turns_day(store, paths, &cfg.device_id, day)?;
        day_snapshots.push(DaySnapshot {
            day: day.clone(),
            usage_rows: usage,
            turn_rows: turns,
        });
    }

    // Sessions: recompute a derived jsonl per favorited dirty session; delete
    // any leftover jsonl for non-favorited dirty sessions (un-favorite local).
    let dirty_sessions = store.dirty_sessions()?;
    let mut recomputed: Vec<SessionCounts> = Vec::with_capacity(dirty_sessions.len());
    let mut removed: Vec<String> = Vec::new();
    for sid in &dirty_sessions {
        let favorited = store
            .get_session_favorited(&cfg.device_id, sid)?
            .unwrap_or(false);
        match decide_snapshot_action(favorited) {
            // favorited ⇒ the snapshot must exist: recompute it from the store.
            SnapshotAction::Write => {
                let count = crate::sessions::session_snapshot::recompute_session_snapshot(
                    store,
                    paths,
                    &cfg.device_id,
                    sid,
                )?;
                recomputed.push(SessionCounts {
                    session_id: sid.clone(),
                    message_rows: count,
                });
            }
            // not favorited ⇒ the snapshot must not exist. Idempotent: a
            // never-favorited session has no file to remove.
            SnapshotAction::Remove => {
                let path = paths.session_snapshot_path(&cfg.device_id, sid);
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
                removed.push(sid.clone());
            }
        }
    }

    // Providers: materialize this device's providers.json from the store,
    // key-stripped (API keys stay in the local DB — the file carries only
    // structure). No dirty flag: the write is byte-stable, so an unchanged
    // store rewrites identical bytes and `commit_and_push` below no-ops.
    crate::provider::sync::write_own_providers(store, paths, &cfg.device_id)?;

    let pushed = commit_and_push(paths, cfg, "cc-one: sync")?;
    if pushed {
        // Push landed ⇒ the recomputed days/sessions are on the remote; drop
        // them so the next push only touches things with fresh local changes.
        // A push failure returns early via `?` above, leaving flags dirty.
        store.clear_dirty_days_if_unchanged(&day_snapshots, &cfg.device_id)?;
        store.clear_dirty_sessions_if_unchanged(&recomputed, &cfg.device_id, &removed)?;
    }
    Ok(pushed)
}

/// Best-effort [`push_usage`] for the exit flush. Standalone is a no-op; a push
/// failure is logged, never propagated.
pub fn push_usage_best_effort(store: &Store, paths: &Paths, cfg: &ConfigData) {
    if !cfg.is_synced() {
        return;
    }
    if let Err(e) = push_usage(store, paths, cfg) {
        eprintln!("[cc-one] usage push failed: {e}");
    }
}
