//! Device registry: the single home for the "device" concept — membership,
//! naming, and the per-device name artifact.
//!
//! Three concerns that used to be scattered across `config` / `db` / `ingest` /
//! `sync` / `commands` are collected here:
//! - **Device-name artifact** (`config/devices_<id>.json`, one file per device):
//!   the cloud registry a device publishes its identity to and reads its peers'
//!   identities from. Carried by the normal Git sync.
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
//! into it. `is_valid_device_id` / `generate_device_id` stay in `config`
//! (bootstrap coupling); this module calls `crate::config::is_valid_device_id`.

use std::collections::HashSet;

use crate::config::{ConfigData, ConfigStore, Paths};
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::library::LibraryForgetAction;
use crate::model::{DeviceArtifact, DeviceInfo};
use crate::synced_doc;

// ---------------- Device-name artifact (one file per device) ----------------

/// Idempotently publish THIS device's identity to `config/devices/<id>.json`
/// (device-name sync ADR). Writes only when the file is missing or its
/// `display_name` is stale, so repeated calls (boot, every sync) don't churn
/// the worktree. `first_seen` is preserved across rewrites. Returns whether a
/// write actually happened.
///
/// No network: the file is merely staged in the worktree — the normal Git sync
/// (`commit_all` + `push`) carries the whole repo, so this file rides along.
pub fn ensure_own_device_artifact(
    paths: &Paths,
    device_id: &str,
    display_name: &str,
) -> AppResult<bool> {
    // Flat layout: repo/config/devices_<id>.json (no devices/ subdir).
    let path = paths.devices_file_path(device_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).ok();
    // Preserve first_seen across rewrites; seed on first publish.
    let first_seen = existing
        .as_deref()
        .and_then(|t| serde_json::from_str::<DeviceArtifact>(t).ok())
        .map(|a| a.first_seen)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let artifact = DeviceArtifact {
        device_id: device_id.to_string(),
        display_name: display_name.to_string(),
        first_seen,
    };
    // Byte-stable via `synced_doc::stable_bytes` (pretty + trailing newline);
    // the trim-end comparison is this domain's idempotent-publish rule — an
    // already-current file is not rewritten, so syncs don't churn the worktree.
    let desired = synced_doc::stable_bytes(&artifact)?;
    if existing.as_deref().map(str::trim_end) == Some(desired.trim_end()) {
        return Ok(false);
    }
    std::fs::write(&path, desired)?;
    Ok(true)
}

/// Parse one device-name artifact from `path`, requiring the id read off its
/// filename to be a valid device id. Best-effort: a stray or broken file yields
/// `None` and is skipped, so one bad entry never blocks the rest from loading
/// (the tolerant read is [`synced_doc::read_json_doc`]). The new-flat and
/// legacy layouts differ only in how that id is taken from the filename; this
/// holds the shared validate-then-read-then-parse that both used to inline.
fn parse_device_artifact(path: &std::path::Path, id: &str) -> Option<DeviceArtifact> {
    if !crate::config::is_valid_device_id(id) {
        return None;
    }
    synced_doc::read_json_doc(path)
}

/// Read every device's identity artifact. Reads the new flat layout
/// (`config/devices_<id>.json`) and, as a read-only fallback, the legacy
/// `config/devices/<id>.json` layout; the flat layout wins on duplicates.
/// Stray or broken files are skipped via `parse_device_artifact`.
pub fn read_all_device_artifacts(paths: &Paths) -> Vec<DeviceArtifact> {
    let mut out: Vec<DeviceArtifact> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // New flat layout: config/devices_<id>.json. Strip the `devices_` prefix
    // and `.json` suffix; the remainder must be a valid device id.
    if let Ok(entries) = std::fs::read_dir(&paths.repo_config) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(id) = name
                .strip_prefix("devices_")
                .and_then(|s| s.strip_suffix(".json"))
            else {
                continue;
            };
            if let Some(a) = parse_device_artifact(&path, id) {
                if seen.insert(a.device_id.clone()) {
                    out.push(a);
                }
            }
        }
    }

    // Legacy layout: config/devices/<id>.json (read-only fallback; new wins).
    if let Ok(entries) = std::fs::read_dir(paths.legacy_devices_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some(a) = parse_device_artifact(&path, stem) {
                if seen.insert(a.device_id.clone()) {
                    out.push(a);
                }
            }
        }
    }

    out
}

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
                .filter(|s| crate::config::is_valid_device_id(s))
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
            .unwrap_or_else(|| crate::config::default_display_name(id));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    /// The Standalone DEGRADATION path of membership: with no usable local git
    /// state (this fixture has no `.git` at `repo/` — Standalone before a first
    /// bind), `present_device_ids` falls back to the worktree approximation,
    /// and `reload_devices_into_store` keeps devices the worktree still backs
    /// (this device, a peer with a registry file, a peer with a data dir) while
    /// purging local-only residue (a device with no presence at all). The
    /// git-backed primary path is pinned by the membership tests below.
    #[test]
    fn reload_devices_reconciles_stale_local_only_devices() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(&paths.repo_config).unwrap();
        std::fs::create_dir_all(&paths.repo_data).unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let self_id = "0123456789ab";
        let live_peer = "aaaaaaaaaaaa"; // backed by a pulled registry file
        let data_peer = "bbbbbbbbbbbb"; // backed by a repo/data/<id>/ dir
        let ghost = "cccccccccccc"; // local-only: no git presence

        // Seed all four into the local registry.
        for id in [self_id, live_peer, data_peer, ghost] {
            store.upsert_device(id, "name", id == self_id).unwrap();
        }
        assert_eq!(store.list_device_ids().unwrap().len(), 4);

        // Git presence after the (simulated) pull.
        ensure_own_device_artifact(&paths, live_peer, "name").unwrap();
        std::fs::create_dir_all(paths.device_data_dir(data_peer)).unwrap();
        // ghost: intentionally nothing in git.

        reload_devices_into_store(&store, &paths, self_id).unwrap();

        let ids = store.list_device_ids().unwrap();
        assert!(ids.iter().any(|i| i == self_id), "self always kept");
        assert!(ids.iter().any(|i| i == live_peer), "registry peer kept");
        assert!(ids.iter().any(|i| i == data_peer), "data-dir peer kept");
        assert!(
            !ids.iter().any(|i| i == ghost),
            "local-only ghost must be pruned"
        );
    }

    /// The pull entry's ORDER INVARIANT (discover before reconcile, pinned
    /// inside `reload_devices_into_store` — the single reconcile trigger), the
    /// two facts that make the order observable after ONE pull:
    ///   - a usage-backed device that IS git-present (a `data/<id>/` subtree
    ///     committed to HEAD) but never published a name artifact gets a
    ///     materialized row and is KEPT — discover's purpose ("appears in the
    ///     picker");
    ///   - a usage-backed device with NO git presence (no artifact blob, no
    ///     data subtree in HEAD — e.g. a peer that deleted itself, a
    ///     regenerated-id residue) is purged IN THE SAME PASS, row AND usage:
    ///     discover first gives reconcile a row to purge. The reverse order
    ///     (reconcile first) would leave it alive — reconcile iterates only
    ///     existing `device` rows, so the not-yet-materialized device would
    ///     survive the pass, show in the picker, and only be purged on a later
    ///     pull.
    #[test]
    fn reload_devices_into_store_pins_discover_before_reconcile() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(&paths.repo_data).unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let self_id = "0123456789ab";
        let peer_usage_only = "aaaaaaaaaaaa"; // usage rows, no artifact, data subtree committed
        let orphan_usage_only = "bbbbbbbbbbbb"; // usage rows, no artifact, NO git presence

        // Usage rows for both; git presence (committed data subtree — git
        // carries files, never empty dirs) only for the peer. Self is
        // registered directly (boot); no config needed — the pull entry takes
        // only the self id.
        store.upsert_device(self_id, "self", true).unwrap();
        let peer_dir = paths.device_data_dir(peer_usage_only);
        std::fs::create_dir_all(&peer_dir).unwrap();
        std::fs::write(peer_dir.join("usage-2026-07-30.jsonl"), "{}\n").unwrap();
        commit_worktree(&paths);
        store
            .ingest(&[
                crate::db::testutil::rec(
                    "u1",
                    "2026-07-13",
                    "glm-5.2",
                    peer_usage_only,
                    100,
                    50,
                    0.0,
                ),
                crate::db::testutil::rec(
                    "u2",
                    "2026-07-14",
                    "glm-5.2",
                    orphan_usage_only,
                    200,
                    80,
                    0.0,
                ),
            ])
            .unwrap();

        reload_devices_into_store(&store, &paths, self_id).unwrap();

        // The git-present usage-only peer: row materialized by discover, kept
        // by reconcile — this is the "appears in the picker" case.
        let ids = store.list_device_ids().unwrap();
        assert!(
            ids.iter().any(|i| i == peer_usage_only),
            "git-present usage-only device kept with a materialized row"
        );
        let row = store
            .list_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.device_id == peer_usage_only)
            .unwrap();
        assert_eq!(
            row.display_name,
            crate::config::default_display_name(peer_usage_only),
            "fallback name from discover"
        );
        assert_eq!(
            row.first_seen, "2026-07-13T10:00:00.000Z",
            "earliest usage timestamp as first_seen"
        );
        // The no-git usage-only device: purged in the SAME pass (discover
        // materialized it, reconcile pruned it). Reverse order would leave it
        // alive for one collect.
        assert!(
            !ids.iter().any(|i| i == orphan_usage_only),
            "usage-only device without git presence purged same-pass"
        );
        assert_eq!(
            store
                .count_logs(&crate::model::UsageFilter {
                    device_scope: Some(orphan_usage_only.into()),
                    ..crate::model::UsageFilter::default()
                })
                .unwrap(),
            0,
            "its usage rows are gone with it"
        );
        assert!(ids.iter().any(|i| i == self_id), "self always kept");
    }

    /// Init a real git repo at `paths.repo` and commit the current worktree —
    /// the test seam for git-backed presence (HEAD tree = the committed
    /// worktree). Call again after worktree edits to commit removals, mirroring
    /// how the production flow lands changes (`commit_all`).
    fn commit_worktree(paths: &Paths) {
        let repo = git2::Repository::init(&paths.repo).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("test", "test@devices.cc-one").unwrap();
        let head = repo.head();
        // Unborn HEAD ⇒ a parentless first commit (creates the branch).
        let oid = match &head {
            Ok(h) => {
                let parent = h.peel_to_commit().unwrap();
                repo.commit(Some("HEAD"), &sig, &sig, "test", &tree, &[&parent])
                    .unwrap()
            }
            Err(_) => repo
                .commit(Some("HEAD"), &sig, &sig, "test", &tree, &[])
                .unwrap(),
        };
        let _ = oid;
    }

    /// THE jitter case the HEAD-tree oracle exists for (ADR-0013): the worktree
    /// lost a device's files — a failed force-checkout, an interrupted rebase,
    /// an external branch switch — while the HEAD tree still carries it.
    /// Reconcile must NOT forget: the device's row AND its usage survive. Under
    /// the old worktree-read membership this exact state wiped a live peer's
    /// local data on the next trigger.
    #[test]
    fn reconcile_keeps_a_device_the_worktree_lost_while_head_still_carries() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let self_id = "0123456789ab";
        let peer = "aaaaaaaaaaaa";
        store.upsert_device(self_id, "self", true).unwrap();
        store.upsert_device(peer, "Peer One", false).unwrap();
        // Peer presence committed to HEAD: data subtree + name artifact.
        let peer_dir = paths.device_data_dir(peer);
        std::fs::create_dir_all(&peer_dir).unwrap();
        std::fs::write(peer_dir.join("usage-2026-07-30.jsonl"), "{}\n").unwrap();
        ensure_own_device_artifact(&paths, peer, "Peer One").unwrap();
        commit_worktree(&paths);
        store
            .ingest(&[crate::db::testutil::rec(
                "u1",
                "2026-07-13",
                "glm-5.2",
                peer,
                100,
                50,
                0.0,
            )])
            .unwrap();

        // The worktree glitch: both per-device traces vanish from the worktree.
        std::fs::remove_dir_all(paths.device_data_dir(peer)).unwrap();
        std::fs::remove_file(paths.devices_file_path(peer)).unwrap();

        reconcile_devices(&store, &paths, self_id).unwrap();

        assert!(
            store.list_device_ids().unwrap().iter().any(|i| i == peer),
            "HEAD still carries the peer — worktree loss is not a departure"
        );
        assert_eq!(
            store
                .count_logs(&crate::model::UsageFilter {
                    device_scope: Some(peer.into()),
                    ..crate::model::UsageFilter::default()
                })
                .unwrap(),
            1,
            "the peer's usage survives the worktree glitch"
        );
    }

    /// Git-truth semantics: once the HEAD tree no longer carries a device (its
    /// removal was committed — the state a peer deleting itself and pushing
    /// leaves behind), reconcile forgets it: registry row AND local footprint.
    #[test]
    fn reconcile_forgets_a_device_head_no_longer_carries() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let self_id = "0123456789ab";
        let peer = "aaaaaaaaaaaa";
        store.upsert_device(self_id, "self", true).unwrap();
        store.upsert_device(peer, "Peer One", false).unwrap();
        let peer_dir = paths.device_data_dir(peer);
        std::fs::create_dir_all(&peer_dir).unwrap();
        std::fs::write(peer_dir.join("usage-2026-07-30.jsonl"), "{}\n").unwrap();
        ensure_own_device_artifact(&paths, peer, "Peer One").unwrap();
        commit_worktree(&paths);
        store
            .ingest(&[crate::db::testutil::rec(
                "u1",
                "2026-07-13",
                "glm-5.2",
                peer,
                100,
                50,
                0.0,
            )])
            .unwrap();

        // The peer deletes itself and pushes: its files leave the worktree and
        // the removal is committed — HEAD no longer carries the device.
        std::fs::remove_dir_all(paths.device_data_dir(peer)).unwrap();
        std::fs::remove_file(paths.devices_file_path(peer)).unwrap();
        commit_worktree(&paths);

        reconcile_devices(&store, &paths, self_id).unwrap();

        assert!(
            !store.list_device_ids().unwrap().iter().any(|i| i == peer),
            "a device HEAD dropped is residue — forgotten"
        );
        assert_eq!(
            store
                .count_logs(&crate::model::UsageFilter {
                    device_scope: Some(peer.into()),
                    ..crate::model::UsageFilter::default()
                })
                .unwrap(),
            0,
            "its local data footprint goes with it"
        );
    }

    /// Worktree presence is NOT membership once git truth is readable: a repo
    /// with a committed baseline plus a peer's files sitting UNCOMMITTED in the
    /// worktree — reconcile still forgets the peer, because HEAD carries
    /// nothing for it. Legitimate peer files always arrive via checkout of
    /// committed content, so uncommitted peer files are transient residue; the
    /// worktree decides only when no usable git state exists (the Standalone
    /// degradation path pinned by
    /// [`reload_devices_reconciles_stale_local_only_devices`]).
    #[test]
    fn reconcile_ignores_worktree_only_presence_once_git_truth_is_readable() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let self_id = "0123456789ab";
        let peer = "aaaaaaaaaaaa";
        store.upsert_device(self_id, "self", true).unwrap();
        store.upsert_device(peer, "Peer One", false).unwrap();
        // A committed baseline (git state readable), then the peer's files
        // land in the worktree WITHOUT a commit.
        std::fs::create_dir_all(&paths.repo).unwrap();
        std::fs::write(paths.repo.join("README"), "seed\n").unwrap();
        commit_worktree(&paths);
        std::fs::create_dir_all(paths.device_data_dir(peer)).unwrap();
        ensure_own_device_artifact(&paths, peer, "Peer One").unwrap();

        reconcile_devices(&store, &paths, self_id).unwrap();

        assert!(
            !store.list_device_ids().unwrap().iter().any(|i| i == peer),
            "uncommitted worktree presence is not git backing"
        );
    }

    #[test]
    fn device_artifact_flat_layout_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Writes to the new flat path (config/devices_<id>.json).
        assert!(ensure_own_device_artifact(&paths, "0123456789ab", "Laptop").unwrap());
        // Idempotent: identical content ⇒ no rewrite.
        assert!(!ensure_own_device_artifact(&paths, "0123456789ab", "Laptop").unwrap());
        // Reads back from the flat path.
        let read = read_all_device_artifacts(&paths);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].device_id, "0123456789ab");
        assert_eq!(read[0].display_name, "Laptop");
        // Path is flat — no legacy devices/ subdir was created.
        assert!(paths.devices_file_path("0123456789ab").exists());
        assert!(!paths
            .legacy_devices_dir()
            .join("0123456789ab.json")
            .exists());
    }

    /// Golden wire bytes for the device-name artifact rewrite: pinned
    /// line-for-line so the shared byte-stable serialization
    /// ([`synced_doc::stable_bytes`]) can never drift this file's format
    /// (pretty JSON + exactly one trailing newline). Seeded stale so the
    /// rewrite (preserving `first_seen`) is deterministic.
    #[test]
    fn ensure_own_device_artifact_lands_pinned_wire_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let file = paths.devices_file_path("0123456789ab");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            r#"{"device_id":"0123456789ab","display_name":"Old","first_seen":"2026-01-01T00:00:00.000Z"}"#,
        )
        .unwrap();
        assert!(ensure_own_device_artifact(&paths, "0123456789ab", "Laptop").unwrap());

        let text = std::fs::read_to_string(&file).unwrap();
        let expected = [
            "{",
            "  \"device_id\": \"0123456789ab\",",
            "  \"display_name\": \"Laptop\",",
            "  \"first_seen\": \"2026-01-01T00:00:00.000Z\"",
            "}",
        ];
        assert_eq!(
            text.lines().collect::<Vec<&str>>(),
            expected,
            "device artifact wire bytes drifted"
        );
        assert!(text.ends_with("}\n"), "exactly one trailing newline");
    }

    #[test]
    fn read_all_device_artifacts_reads_legacy_layout_too() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Seed a legacy file under config/devices/<id>.json (old layout peer).
        let legacy = paths.legacy_devices_dir().join("abcdef012345.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(
            &legacy,
            r#"{"device_id":"abcdef012345","display_name":"OldPeer","first_seen":"2026-01-01T00:00:00.000Z"}"#,
        )
        .unwrap();
        // And a flat file for a different device (new layout).
        ensure_own_device_artifact(&paths, "0123456789ab", "NewPeer").unwrap();

        let mut ids: Vec<String> = read_all_device_artifacts(&paths)
            .into_iter()
            .map(|a| a.device_id)
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["0123456789ab".to_string(), "abcdef012345".to_string()],
            "both layouts are read"
        );
    }

    /// `apply_aliases` re-derives `is_self` from the live config and overlays
    /// local aliases on top of the synced names, leaving un-aliased devices'
    /// names untouched.
    #[test]
    fn apply_aliases_layers_local_names_and_rederives_self() {
        let mut devices = vec![
            DeviceInfo {
                device_id: "0123456789ab".into(),
                display_name: "Synced Self".into(),
                // Stale stored value — must be corrected.
                is_self: false,
                first_seen: String::new(),
            },
            DeviceInfo {
                device_id: "aaaaaaaaaaaa".into(),
                display_name: "Synced Peer".into(),
                is_self: true, // Stale — a peer mislabeled as self.
                first_seen: String::new(),
            },
            DeviceInfo {
                device_id: "bbbbbbbbbbbb".into(),
                display_name: "Other Peer".into(),
                is_self: false,
                first_seen: String::new(),
            },
        ];
        let mut cfg = ConfigData {
            device_id: "0123456789ab".into(),
            ..Default::default()
        };
        cfg.device_names
            .insert("aaaaaaaaaaaa".to_string(), "Aliased Peer".to_string());

        apply_aliases(&mut devices, &cfg);

        assert!(devices[0].is_self, "self re-derived from live cfg");
        assert_eq!(devices[0].display_name, "Synced Self", "self name kept");
        assert!(!devices[1].is_self, "peer no longer mislabeled as self");
        assert_eq!(
            devices[1].display_name, "Aliased Peer",
            "alias wins over synced name"
        );
        assert!(!devices[2].is_self);
        assert_eq!(
            devices[2].display_name, "Other Peer",
            "un-aliased device keeps its synced name"
        );
    }

    /// `register_self` seeds the Local Store row and publishes the name
    /// artifact — the two boot-time writes that used to be inlined in `lib.rs`.
    fn config_store_at(root: &std::path::Path, data: ConfigData) -> crate::config::ConfigStore {
        crate::config::ConfigStore::for_test(Paths::resolve(root), data)
    }

    #[test]
    fn register_self_writes_store_row_and_name_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(&paths.repo_config).unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let cfg = ConfigData {
            device_id: "0123456789ab".into(),
            display_name: "Laptop".into(),
            ..Default::default()
        };
        let config = config_store_at(tmp.path(), cfg);

        register_self(&store, &config).unwrap();

        // Store row seeded as self.
        let devices = store.list_devices().unwrap();
        let row = devices
            .iter()
            .find(|d| d.device_id == "0123456789ab")
            .unwrap();
        assert_eq!(row.display_name, "Laptop");
        assert!(row.is_self);
        // Name artifact published to the flat path.
        let arts = read_all_device_artifacts(&paths);
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].device_id, "0123456789ab");
        assert_eq!(arts[0].display_name, "Laptop");
    }

    #[test]
    fn rename_self_updates_config_store_row_and_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(&paths.repo_config).unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let cfg = ConfigData {
            device_id: "0123456789ab".into(),
            display_name: "Old".into(),
            ..Default::default()
        };
        let config = config_store_at(tmp.path(), cfg);
        register_self(&store, &config).unwrap();

        rename_self(&store, &config, "New Name").unwrap();

        // Local config reflects the new name.
        assert_eq!(config.get().display_name, "New Name");
        // Store row carries the new name.
        let row = store
            .list_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.device_id == "0123456789ab")
            .unwrap();
        assert_eq!(row.display_name, "New Name");
        // Artifact republished with the new name.
        let arts = read_all_device_artifacts(&paths);
        assert_eq!(arts.len(), 1);
        assert_eq!(arts[0].display_name, "New Name");
    }

    /// `rename_peer` records a local alias and upserts the Store row, re-deriving
    /// `is_self` from the live config so a peer is never mislabeled.
    #[test]
    fn rename_peer_sets_alias_and_store_row() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();
        let cfg = ConfigData {
            device_id: "0123456789ab".into(),
            ..Default::default()
        };
        let config = config_store_at(tmp.path(), cfg);

        rename_peer(&store, &config, "aaaaaaaaaaaa", "Peer One").unwrap();

        assert_eq!(
            config
                .get()
                .device_names
                .get("aaaaaaaaaaaa")
                .map(String::as_str),
            Some("Peer One"),
            "alias recorded locally"
        );
        let row = store
            .list_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.device_id == "aaaaaaaaaaaa")
            .unwrap();
        assert_eq!(row.display_name, "Peer One");
        assert!(!row.is_self, "is_self re-derived false for a peer");
    }

    /// `forget_device` drops everything local that named the peer: registry row,
    /// alias, Artifact dir, name artifact, and (under Delete) the library
    /// subtree. Best-effort steps must not abort the rest.
    #[test]
    fn forget_device_drops_row_alias_data_artifact_and_library() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(&paths.repo_config).unwrap();
        std::fs::create_dir_all(&paths.repo_data).unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let peer = "aaaaaaaaaaaa";
        let mut cfg = ConfigData {
            device_id: "0123456789ab".into(),
            ..Default::default()
        };
        cfg.device_names.insert(peer.into(), "Old Peer".into());
        let config = config_store_at(tmp.path(), cfg);

        // Seed: Store row + alias (in cfg) + data dir + name artifact + library.
        store.upsert_device(peer, "Old Peer", false).unwrap();
        std::fs::create_dir_all(paths.device_data_dir(peer)).unwrap();
        ensure_own_device_artifact(&paths, peer, "Old Peer").unwrap();
        let peer_lib = paths.library.join(peer);
        std::fs::create_dir_all(&peer_lib).unwrap();
        std::fs::write(peer_lib.join("note.txt"), "hi").unwrap();

        forget_device(
            &store,
            &config,
            &paths,
            peer,
            crate::library::LibraryForgetAction::Delete,
        )
        .unwrap();

        assert!(
            !store.list_device_ids().unwrap().iter().any(|i| i == peer),
            "peer registry row dropped"
        );
        assert!(
            !config.get().device_names.contains_key(peer),
            "alias cleared"
        );
        assert!(!paths.device_data_dir(peer).exists(), "data dir removed");
        assert!(
            !paths.devices_file_path(peer).exists(),
            "name artifact removed"
        );
        assert!(!peer_lib.exists(), "library subtree deleted");
        // The captured alias (removed from the map by the forget) named the
        // migrate folder on Migrate; under Delete it is simply dropped.
    }

    /// The `is_self` guard lives at the domain entry: forgetting THIS device
    /// is a hard error taken BEFORE any drop, so a caller cannot erase this
    /// device's own footprint.
    #[test]
    fn forget_device_rejects_self() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(&paths.repo_config).unwrap();
        let store = crate::db::Store::open(std::path::Path::new(":memory:")).unwrap();

        let self_id = "0123456789ab";
        let config = config_store_at(
            tmp.path(),
            ConfigData {
                device_id: self_id.into(),
                ..Default::default()
            },
        );
        store.upsert_device(self_id, "self", true).unwrap();

        let err = forget_device(
            &store,
            &config,
            &paths,
            self_id,
            crate::library::LibraryForgetAction::Delete,
        );
        assert!(err.is_err(), "this device is never forgettable");
        assert!(
            store
                .list_device_ids()
                .unwrap()
                .iter()
                .any(|i| i == self_id),
            "the guard fires before anything is dropped"
        );
    }
}
