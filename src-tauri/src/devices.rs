//! Device registry: the single home for the "device" concept — membership,
//! naming, and the per-device name artifact.
//!
//! Three concerns that used to be scattered across `config` / `db` / `ingest` /
//! `sync` / `commands` are collected here:
//! - **Device-name artifact** (`config/devices_<id>.json`, one file per device):
//!   the cloud registry a device publishes its identity to and reads its peers'
//!   identities from. Carried by the normal Git sync. ([`artifact`])
//! - **Membership**: "which devices exist" is decided by git itself — the local
//!   HEAD tree (a `config/devices_<id>.json` blob or a `data/<id>/` subtree,
//!   via `crate::sync::head_tree_device_ids`) — and reconciled against the
//!   Local Store's `device` table (stale local-only rows pruned). The worktree
//!   is only the degradation path for when no usable git state exists
//!   (Standalone, unborn HEAD). The destructive reconcile runs ONLY at the
//!   single post-pull point ([`reload_devices_into_store`]); the collect
//!   heartbeat does non-destructive maintenance only.
//! - **Naming**: local aliases (set via `set_device_display_name`) are layered
//!   over the synced names at read time.
//!
//! The `device` table CRUD itself (`upsert_device` / `list_devices` /
//! `list_device_ids` / `forget_device_local` / `discover_devices_from_usage`)
//! stays in `db::Store`; this module is the registry orchestrator that calls
//! into it. The id primitives (`is_valid_device_id` / `generate_device_id` /
//! `default_display_name`) live in [`id`] — "which ids are device ids and what
//! does a fresh one look like" is registry knowledge — and `config`'s
//! bootstrap (`ConfigStore::load_at`) calls into them for first-generation;
//! `sync::git` and `db::store_devices` consume them via `crate::devices::`
//! directly.

use std::collections::HashSet;

use crate::config::{ConfigData, ConfigStore, Paths};
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::library::LibraryForgetAction;
use crate::model::DeviceInfo;

mod artifact;
mod id;
#[cfg(test)]
mod tests;

pub use artifact::{ensure_own_device_artifact, read_all_device_artifacts};
pub(crate) use id::generate_device_id;
pub use id::{default_display_name, is_valid_device_id};

// ---------------- Membership ----------------

/// Iterate the valid device-id directory names under `repo/data/` — the shared
/// "walk the per-device data dirs" loop that artifact reading, session
/// snapshots, synced-group reading, and membership each used to inline. Stray
/// non-device folders and non-directory entries are skipped by the id shape.
/// Absent `repo/data/` ⇒ empty; a read error propagates. Sorted for stable,
/// display-friendly ordering.
pub fn iter_data_device_ids(paths: &Paths) -> AppResult<Vec<String>> {
    let root = &paths.repo_data;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut ids: Vec<String> = std::fs::read_dir(root)?
        .flatten()
        .filter_map(|e| {
            if !e.file_type().ok()?.is_dir() {
                return None;
            }
            e.file_name()
                .to_str()
                .filter(|s| is_valid_device_id(s))
                .map(|s| s.to_string())
        })
        .collect();
    ids.sort();
    Ok(ids)
}

/// The set of device ids the local repo still backs — THE membership set. Git
/// is the source of truth, and the truth read here is git ITSELF (the local
/// HEAD tree via [`crate::sync::head_tree_device_ids`]: name artifacts and
/// per-device data subtrees, committed state), not the worktree: a failed
/// force-checkout, an interrupted rebase, or an external branch switch can
/// transiently empty `repo/data/<peer>/` while HEAD still carries it, and such
/// jitter must never read as a device leaving (the destructive reconcile
/// forgets absent devices — ADR-0013). Degrades to the worktree approximation
/// (self ∪ worktree name artifacts ∪ worktree `data/<id>/` dirs) only when no
/// usable local git state exists — Standalone (no `.git`), an unborn HEAD, or
/// a failed git read: forget is destructive, so an unreadable truth falls back
/// to the old lenient source, never to "nobody present". Self is always
/// present.
fn present_device_ids(paths: &Paths, self_device_id: &str) -> HashSet<String> {
    let mut present: HashSet<String> = HashSet::new();
    present.insert(self_device_id.to_string());
    if let Some(git_ids) = crate::sync::head_tree_device_ids(&paths.repo) {
        present.extend(git_ids);
        return present;
    }
    // No usable git state (Standalone / unborn HEAD / unreadable): the
    // worktree is the only membership signal left.
    for a in read_all_device_artifacts(paths) {
        present.insert(a.device_id.clone());
    }
    for id in iter_data_device_ids(paths).unwrap_or_default() {
        present.insert(id);
    }
    present
}

/// The device ids the local repo still backs, as an ordered list with this
/// device first — the single enumeration for callers that walk per-device
/// subtrees (e.g. `library::scan`'s "all" scope). Same membership basis as the
/// private `present_device_ids` (git's HEAD tree, worktree only as the
/// Standalone degradation); ordered here because subtree walks are displayed,
/// self first.
pub fn known_device_ids(paths: &Paths, cfg: &ConfigData) -> Vec<String> {
    let mut ids: Vec<String> = present_device_ids(paths, &cfg.device_id)
        .into_iter()
        .collect();
    ids.sort_by_key(|id| (id != &cfg.device_id, id.clone()));
    ids
}

// ---------------- Registry reconciliation ----------------

/// Refresh the device registry on the collect path — the collect-side
/// maintenance entry, NON-DESTRUCTIVE by design:
///   1. [`touch_self`] — keep THIS device's row current (a rename self-heals);
///   2. `Store::discover_devices_from_usage` — materialize rows for devices
///      that have usage but no row yet (picker latency ≤ one collect interval).
///
/// Reconcile (the purge of rows git no longer backs) deliberately does NOT run
/// here: it deletes the forgotten device's entire local footprint, and the
/// collect heartbeat must never be able to fire that off a misread — presence
/// read from the worktree once let a transient FS glitch (failed
/// force-checkout, interrupted rebase, external branch switch) wipe a live
/// peer's local data on the very next collect tick. The destructive half lives
/// at the single post-pull point ([`reload_devices_into_store`]), where
/// presence was just re-established by a successful fetch + checkout
/// (ADR-0013).
pub fn refresh_device_registry(store: &Store, cfg: &ConfigData) -> AppResult<()> {
    touch_self(store, cfg)?;
    store.discover_devices_from_usage()
}

/// Purge local device rows git no longer backs. Git is the source of truth for
/// which devices exist, so a device with NO git presence is residue and is
/// forgotten locally (registry row + its full data footprint: usage, turns,
/// session rows, transcript messages). "Present" = this device ∪ devices git's
/// local HEAD tree still carries (a `config/devices_<id>.json` name artifact
/// or a `data/<id>/` subtree — committed state, via [`present_device_ids`],
/// which degrades to the worktree only when no usable local git state exists).
/// Runs ONLY at the single post-pull point ([`reload_devices_into_store`]) —
/// never on the collect heartbeat — so worktree/git jitter can never reach the
/// destructive path, and a purge always rides a pull that just confirmed what
/// git carries. `self_device_id` is always kept. A failure on one id is
/// logged, not fatal.
pub fn reconcile_devices(store: &Store, paths: &Paths, self_device_id: &str) -> AppResult<()> {
    // Build the set of devices git still backs.
    let present = present_device_ids(paths, self_device_id);

    // Purge dirty rows: local-only devices git no longer backs. Self is always
    // kept (it's in `present`). A failure on one id is logged, not fatal.
    for id in store.list_device_ids()? {
        if id == self_device_id || present.contains(&id) {
            continue;
        }
        match store.forget_device_local(&id) {
            Ok(n) => eprintln!("[cc-one] reconciled stale device {id} ({n} rows dropped)"),
            Err(e) => eprintln!("[cc-one] failed to reconcile device {id}: {e}"),
        }
    }
    Ok(())
}

/// Reload the (just-pulled) cloud device registry into the Store, then run the
/// registry's destructive maintenance — the SINGLE reconcile trigger. Each
/// registry file upsert is best-effort so one bad row can't abort the rest.
/// Aliases stay local and are layered on at `list_devices`. Used by the sync
/// pull path as the devices domain's import (the `sync::domains::DOMAINS`
/// table points here); running inside the pull is the point: presence was just
/// re-established by a successful fetch + checkout, so a purge decided here
/// reflects what git actually carries, and a transient worktree/git glitch on
/// the collect heartbeat can never fire a forget (ADR-0013). Takes only
/// `self_device_id` — this half of the registry never needs the rest of the
/// config.
///
/// THE ORDER IS LOAD-BEARING: `Store::discover_devices_from_usage` must run
/// BEFORE [`reconcile_devices`], so rows discover just materialized are
/// reconciled against git truth IN THE SAME PASS — a usage-only device whose
/// repo presence is gone (a peer deleted itself, a regenerated-id residue) is
/// purged immediately, together with its usage, instead of lingering in the
/// picker until a later pass purges it. (Reconcile iterates the store's
/// `device` table, so a usage-backed device that has NO row yet is only
/// purgeable once discover has materialized it — discover-first is what makes
/// the same-pass cleanup happen.) Pinned inside this one entry so the pull
/// path cannot reorder the steps; the invariant is unit-tested here
/// ([`tests::reload_devices_into_store_pins_discover_before_reconcile`]).
///
/// Returns the number of registry rows loaded from the pulled name artifacts
/// (the devices domain's `imported` count; a re-pull recounts unchanged rows —
/// the upsert dedupes, the count reports volume, not novelty).
pub(crate) fn reload_devices_into_store(
    store: &Store,
    paths: &Paths,
    self_device_id: &str,
) -> AppResult<u32> {
    let artifacts = read_all_device_artifacts(paths);
    let loaded = artifacts.len() as u32;
    for a in artifacts {
        // The `is_self` rule (self id comparison, re-derived per call site).
        let is_self = a.device_id == self_device_id;
        if let Err(e) = store.upsert_device(&a.device_id, &a.display_name, is_self) {
            eprintln!("[cc-one] device reload skipped {}: {e}", a.device_id);
        }
    }
    store.discover_devices_from_usage()?;
    reconcile_devices(store, paths, self_device_id)?;
    Ok(loaded)
}

// ---------------- Naming layer ----------------

/// Whether `device_id` is THIS device, per the live config. Re-derived at every
/// call site (never trusted from a stored column) because this device's id can
/// be regenerated — a peer must never be mislabeled "this device". The single
/// rule every site derives from, so the comparison can't drift between them.
pub fn is_self(cfg: &ConfigData, device_id: &str) -> bool {
    cfg.device_id == device_id
}

/// The display name for one device: a local alias (set via
/// `set_device_display_name`) wins where present, the synced name otherwise.
/// This is the SINGLE resolution rule — both [`apply_aliases`] (batch, over
/// `DeviceInfo` rows) and [`resolve_display_names`] (per-device, for callers
/// without the rows) route through it, so a device's name can never diverge
/// between the dashboard list and the library view.
fn layer_alias(cfg: &ConfigData, device_id: &str, synced_name: &str) -> String {
    cfg.device_names
        .get(device_id)
        .cloned()
        .unwrap_or_else(|| synced_name.to_string())
}

/// Resolve a display name for each id in `device_ids`, layering local aliases
/// over the synced name from the registry. A device with no registry row (e.g.
/// a data dir on disk before the row is discovered) falls back to the default
/// generated name — never a raw id — so the library view can't diverge from the
/// dashboard. The single resolution for callers without `DeviceInfo` rows.
pub fn resolve_display_names(
    store: &Store,
    cfg: &ConfigData,
    device_ids: &[String],
) -> AppResult<std::collections::HashMap<String, String>> {
    let synced: std::collections::HashMap<String, String> = store
        .list_devices()?
        .into_iter()
        .map(|d| (d.device_id, d.display_name))
        .collect();
    let mut out = std::collections::HashMap::new();
    for id in device_ids {
        let name = synced
            .get(id)
            .cloned()
            .unwrap_or_else(|| default_display_name(id));
        out.insert(id.clone(), layer_alias(cfg, id, &name));
    }
    Ok(out)
}

/// Layer local aliases over the synced device names, and re-derive `is_self`
/// from the live config. Mutates `devices` in place. Thin batch wrapper over
/// [`is_self`] + [`layer_alias`] — the single resolution rule.
pub fn apply_aliases(devices: &mut [DeviceInfo], cfg: &ConfigData) {
    for d in devices {
        d.is_self = is_self(cfg, &d.device_id);
        d.display_name = layer_alias(cfg, &d.device_id, &d.display_name);
    }
}

// ---------------- Lifecycle: register / rename / forget ----------------

/// Refresh THIS device's registry row. Idempotent (UPSERT); routed here — not a
/// bare `store.upsert_device` at the call site — so every self-row write goes
/// through the registry orchestrator, alongside `register_self` (boot) and
/// `rename_self`. Used on the collect path to keep the row current as the user
/// renames this device.
pub fn touch_self(store: &Store, cfg: &ConfigData) -> AppResult<()> {
    store.upsert_device(&cfg.device_id, &cfg.display_name, true)
}

/// Register THIS device on boot: a row in the Local Store and the published
/// name artifact. Both best-effort — boot must not fail on these (the original
/// boot block ran two independent `let _ =`). The Store row is authoritative
/// and self-heals on the next rename; the artifact write self-heals on the
/// next sync. Idempotent: safe on every boot.
pub fn register_self(store: &Store, config: &ConfigStore) -> AppResult<()> {
    let cfg = config.get();
    let _ = store.upsert_device(&cfg.device_id, &cfg.display_name, true);
    let _ = ensure_own_device_artifact(&config.paths(), &cfg.device_id, &cfg.display_name);
    Ok(())
}

/// Rename THIS device (display name only — not a uniqueness key): update local
/// config, refresh the Store row, and republish the name artifact. Config +
/// Store are hard errors (a half-applied rename would split the registry); the
/// artifact write is best-effort (a failure doesn't undo the local rename, and
/// the file self-heals on the next sync — `ensure_own_device_artifact` is a
/// no-op when the file is already current).
pub fn rename_self(store: &Store, config: &ConfigStore, new_name: &str) -> AppResult<()> {
    let cfg = config.update(|c| {
        c.display_name = new_name.to_string();
    })?;
    store.upsert_device(&cfg.device_id, &cfg.display_name, true)?;
    let _ = ensure_own_device_artifact(&config.paths(), &cfg.device_id, &cfg.display_name);
    Ok(())
}

/// Set a friendly name for a device (self or peer): upsert the Store row and
/// record a local alias. Aliases are local-only (never synced); they layer
/// over synced names at read time via [`apply_aliases`]. `is_self` is
/// re-derived from the live config so the Store column can never mislabel a
/// peer as "this device". Named `rename_peer` after the primary use case
/// (naming a peer seen in the repo), but a self-id is handled correctly too.
pub fn rename_peer(
    store: &Store,
    config: &ConfigStore,
    device_id: &str,
    display_name: &str,
) -> AppResult<()> {
    let is_self = is_self(&config.get(), device_id);
    store.upsert_device(device_id, display_name, is_self)?;
    config.update(|c| {
        c.device_names
            .insert(device_id.to_string(), display_name.to_string());
    })?;
    Ok(())
}

/// Delete a device's per-device Artifact dir `repo/data/<id>/` (best-effort:
/// a missing dir is a no-op; an error is logged). Local-only — no Git push; a
/// peer still in the repo reappears on the next sync.
fn remove_device_data_dir(paths: &Paths, device_id: &str) {
    let dir = paths.device_data_dir(device_id);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            eprintln!(
                "[cc-one] forget_device: failed to remove {}: {e}",
                dir.display()
            );
        }
    }
}

/// Delete a device's published name artifact `repo/config/devices_<id>.json`
/// (best-effort: a missing file is a no-op; an error is logged). This module
/// already owns the read + write of that file; this closes the delete side of
/// the trio.
fn remove_device_artifact_file(paths: &Paths, device_id: &str) {
    let file = paths.devices_file_path(device_id);
    if file.exists() {
        if let Err(e) = std::fs::remove_file(&file) {
            eprintln!(
                "[cc-one] forget_device: failed to remove {}: {e}",
                file.display()
            );
        }
    }
}

/// Locally forget a peer device: drop its registry row + all its local data
/// footprint (usage records, turn durations, session rows, transcript
/// messages and their pending-dirty flags), clear any local alias,
/// delete its Artifact dir and published name artifact, and apply
/// [`LibraryForgetAction`] to its library subtree. The Store + alias removals
/// are hard errors (a half-forgotten device would leave the registry
/// inconsistent); the filesystem + library cleanups are best-effort (logged,
/// not propagated — a peer still in the repo reappears on the next sync, so a
/// leftover dir/file self-heals). Nothing is pushed to Git.
///
/// 回灌语义（git 同步固有，非缺陷）：对端数据由 git 快照承载，本机删行不是
/// 数据丢失——但「遗忘一个仍活跃于仓库的对端」也不持久：其快照文件还在
/// 远端，下一次 pull 的 fast-forward 强制检出会把它们带回工作树，
/// `sessions_import` / `usage_import` 随之全量重建其会话行与用量（实际行为
/// 由 `sync::domains` 测试块的
/// `forget_device_local_is_undone_by_a_pull_that_restores_the_snapshot`
/// 无 git 固化）。遗忘要落地，靠下一次 commit_all 把工作树删除提交推上远端。
///
/// The peer's migrate-target folder is named after its LOCAL alias, captured
/// here BEFORE the alias map is dropped (`from-<name>`; an empty alias falls
/// back to the device id, per `library`'s folder-name rule) — so callers hand
/// over only `(device_id, library_action)` and cannot forget to snapshot the
/// name first. Self is NOT forgettable: the `is_self` guard lives HERE, at
/// the registry lifecycle entry (this device is renamed, never forgotten), so
/// no caller can bypass it. `db::Store::forget_device_local` keeps its own
/// caller-guard contract because the Store has no config access to check
/// with; this layer does.
pub fn forget_device(
    store: &Store,
    config: &ConfigStore,
    paths: &Paths,
    device_id: &str,
    library_action: LibraryForgetAction,
) -> AppResult<()> {
    let cfg = config.get();
    // A hard error BEFORE anything is dropped — forgetting self would erase
    // this device's own footprint.
    if is_self(&cfg, device_id) {
        return Err(AppError::Config(
            "this device cannot be removed (rename it instead)".into(),
        ));
    }
    // Capture the peer's alias before the alias map is dropped (see doc).
    let peer_name = cfg.device_names.get(device_id).cloned().unwrap_or_default();
    // Hard errors: registry row + alias map. Abort the whole forget if either
    // fails — a half-forgotten device would leave the registry inconsistent.
    store.forget_device_local(device_id)?;
    config.update(|c| {
        c.device_names.remove(device_id);
    })?;
    // Best-effort FS cleanups this module owns: the Artifact dir + name file.
    remove_device_data_dir(paths, device_id);
    remove_device_artifact_file(paths, device_id);
    // Best-effort library cleanup (migrate or delete). Local-only — no Git push.
    // Delegated to `library`: it owns the library subtree shape. `library` does
    // not depend on `devices`, so this reverse call is not a cycle.
    if let Err(e) = crate::library::forget_device_library(
        paths,
        &config.get(),
        device_id,
        library_action,
        &peer_name,
    ) {
        eprintln!(
            "[cc-one] forget_device: library {:?} failed: {e}",
            library_action
        );
    }
    Ok(())
}
