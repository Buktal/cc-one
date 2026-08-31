//! devices 域测试：id 原语、membership（git HEAD tree 真相源 + Standalone
//! 降级）、reconcile 顺序不变量、name artifact 读写、命名分层与 lifecycle。

use super::*;
use crate::config::Paths;
use std::fs;

// ---- id 原语：校验 / 首代（随 generate_device_id 从 config 搬来）----

#[test]
fn valid_device_id_rules() {
    assert!(is_valid_device_id("0123456789ab"));
    assert!(is_valid_device_id("abcdef012345"));
    assert!(!is_valid_device_id("0123456789a")); // too short
    assert!(!is_valid_device_id("0123456789abc")); // too long
    assert!(!is_valid_device_id("abcdef01234g")); // non-hex letter
    assert!(!is_valid_device_id("ABCDEF012345")); // uppercase rejected
}

#[test]
fn generated_device_id_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    let id = generate_device_id(&paths);
    assert!(is_valid_device_id(&id));
}

#[test]
fn generated_device_id_avoids_existing_collisions() {
    let tmp = tempfile::tempdir().unwrap();
    let paths = Paths::resolve(tmp.path());
    // Pre-seed an existing device dir under repo/data/.
    fs::create_dir_all(paths.device_data_dir("aabbccddeeff")).unwrap();
    for _ in 0..16 {
        let id = generate_device_id(&paths);
        assert_ne!(
            id, "aabbccddeeff",
            "generator must avoid existing device dirs"
        );
        assert!(is_valid_device_id(&id));
    }
}

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
            crate::db::testutil::rec("u1", "2026-07-13", "glm-5.2", peer_usage_only, 100, 50, 0.0),
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
        default_display_name(peer_usage_only),
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
