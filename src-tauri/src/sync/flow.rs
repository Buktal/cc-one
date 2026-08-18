//! High-level sync flow: compose the low-level git primitives (`super::git`)
//! with the per-domain sync pairs (`super::domains`) to form the full
//! pull → import → commit → push pipeline. This is the only layer that knows
//! about git — `super::git` stays pure git, the store stays pure SQL, and
//! `super::domains` owns each syncable domain's materialize/import pair, so
//! adding a domain never touches this file beyond one import call per side.
//! Best-effort wrappers (`*_best_effort`) swallow errors for background/exit
//! paths; their fallible counterparts propagate.

use super::git::{
    author_email, commit_all, has_changes, is_ahead_of_origin, open_or_clone, pull, push,
    rebase_and_push, require_synced, PullOutcome,
};
use crate::config::{ConfigData, Paths};
use crate::db::Store;
use crate::error::AppResult;

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
    // Each syncable domain declares its pull-side import in `super::domains` —
    // usage/turns Artifacts, session snapshots (+ un-favorite propagation),
    // key-stripped provider structure, and the device-name registry.
    let inserted = super::domains::usage_import(store, paths)?;
    super::domains::sessions_import(store, paths, &cfg.device_id)?;
    super::domains::providers_import(store, paths, &cfg.device_id)?;
    super::domains::devices_import(store, paths, cfg)?;
    Ok(inserted)
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
/// commit + push, clearing the dirty flags once the materialization is on the
/// remote. This is the push-side counterpart to collect's store-only
/// writes: collect flags days/sessions dirty; this recomputes each dirty day's
/// per-day Artifact and each dirty session's jsonl snapshot, commits the
/// rewritten files, pushes, and clears the flags (a failed push leaves them
/// dirty for the next retry). Synced-only; a no-op (`false`) when there is
/// nothing dirty to recompute and nothing else to push.
///
/// The session favorites gate lives in `super::domains::sessions_materialize`
/// (not in collect): a favorited dirty session gets its snapshot rewritten; a
/// non-favorited dirty session gets any leftover `sessions/<id>.jsonl` removed
/// — the local half of un-favorite propagation (a peer pulling sees the file
/// vanish). The clear is scoped to recompute-time row/message counts so a
/// raced new row/message keeps its day/session dirty (see
/// [`crate::db::Store::clear_dirty_flags_if_unchanged`]).
///
/// The clear runs even on a no-op push (NOT gated on `pushed`): its
/// if-unchanged guards make a stale clear a no-op, and clearing on a no-op
/// push self-heals the split-clear crash window — a process dying between the
/// commit and the clear leaves flags stale with a worktree that already
/// matches origin, and the next push would otherwise skip the clear forever
/// (`pushed == false` ⇒ stale flags). Days and sessions clear in ONE
/// transaction: a mid-clear failure rolls back both, so the store can never
/// sit in days-clean + sessions-dirty.
///
/// Library sync does NOT call this — it has no store/dirty concern and uses
/// [`commit_and_push`] directly.
pub fn push_usage(store: &Store, paths: &Paths, cfg: &ConfigData) -> AppResult<bool> {
    // Each syncable domain declares its push-side materialize in
    // `super::domains`; the recompute-time snapshots feed the shared clear.
    let day_snapshots = super::domains::usage_materialize(store, paths, &cfg.device_id)?;
    let sessions = super::domains::sessions_materialize(store, paths, &cfg.device_id)?;
    super::domains::providers_materialize(store, paths, &cfg.device_id)?;

    let pushed = commit_and_push(paths, cfg, "cc-one: sync")?;
    // Unconditional (see the doc above). A push failure returns early via `?`
    // above, leaving both flag sets dirty for the next retry.
    store.clear_dirty_flags_if_unchanged(
        &cfg.device_id,
        &day_snapshots,
        &sessions.recomputed,
        &sessions.removed,
    )?;
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
