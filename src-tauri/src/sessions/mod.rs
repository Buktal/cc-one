//! Synced-groups persistence — the domain behind the group half of the
//! sessions Tauri commands (the command layer itself lives in `commands`).
//!
//! This module's body is the Synced track's storage:
//! `data/<deviceId>/groups.json` — the four `*_owned` mutations (each ending
//! in a best-effort commit + push), the merged cross-device read, and the
//! unified `SessionGroup` DTO assembly. Each device writes ONLY its own file;
//! reading merges every device's file by id (the device-registry pattern). Ids
//! carry a device prefix (`<deviceId>-<8hex>`) so they are globally unique
//! without coordination. The per-device-doc mechanism — tolerant read,
//! byte-stable write, latest-wins merge — lives in [`crate::synced_doc`];
//! this module declares the wire doc ([`SyncedGroupsDoc`]) and the domain
//! rules (merge key = id, no skip-self: the file is the authoritative
//! storage).
//!
//! NOT here: session/user-data CRUD (favorited / custom_title / group
//! membership / list / transcript read) — that lives on `db::Store` (sessions
//! table) + `collect::ingest` (transcript I/O); the Local track's groups too
//! (the `local_groups` SQLite table, device-private, never in git) are plain
//! `db::Store` methods. This module contributes only the local-group id
//! generator. The `commands` module calls these `*_owned` operations (and the
//! Store's) and emits `"sessions_changed"` so the frontend refreshes its
//! queries.

pub mod session_snapshot;
pub mod snapshot_policy;

use std::path::PathBuf;

use crate::config::{ConfigData, Paths};
use crate::error::{AppError, AppResult};
use crate::model::{SessionGroup, SyncedGroup};
use crate::synced_doc;

/// The git commit message for every synced-groups change — one domain
/// constant, so the log reads "cc-one: groups sync" no matter which entry
/// pushed.
const GROUPS_SYNC_MSG: &str = "cc-one: groups sync";

/// Per-device synced-groups file: `repo/data/<deviceId>/groups.json`.
fn groups_json_path(paths: &Paths, device_id: &str) -> PathBuf {
    paths.device_data_dir(device_id).join("groups.json")
}

/// The groups wire doc: a stable JSON object with one `groups` array. The
/// wrapper struct IS this domain's wire declaration (field names and order
/// serialize into the file); the mechanism around it — tolerant read,
/// byte-stable write, latest-wins merge — lives in [`crate::synced_doc`]. No
/// schema `v` yet: one shape so far, and writing a constant `v` would gate
/// nothing while rewriting every device's file. The version-gate primitive is
/// ready there if a second shape ever ships.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SyncedGroupsDoc {
    #[serde(default)]
    groups: Vec<SyncedGroup>,
}

/// Read one device's synced-groups file. Missing/unreadable/unparseable ⇒
/// empty — a corrupt peer file must never abort the merged read.
fn read_device_synced_groups(paths: &Paths, device_id: &str) -> Vec<SyncedGroup> {
    synced_doc::read_json_doc::<SyncedGroupsDoc>(&groups_json_path(paths, device_id))
        .unwrap_or_default()
        .groups
}

/// Every device's synced groups merged by id (latest `updated_at` wins; ties →
/// first-seen — [`synced_doc::merge_latest_wins`]; the key and the sort below
/// are this domain's rules). Iterates only valid device dirs so a stray folder
/// never shows up as a groups source. This is the read-side of the
/// per-device-write pattern (mirrors `devices::read_all_device_artifacts`).
/// NOT skipping self (unlike providers / session snapshots): groups.json IS
/// the authoritative storage — there is no DB copy — so this device reads its
/// own file back like a peer's.
pub fn read_all_synced_groups(paths: &Paths) -> Vec<SyncedGroup> {
    let ids = crate::devices::iter_data_device_ids(paths).unwrap_or_default();
    let mut merged = synced_doc::merge_latest_wins(
        synced_doc::read_all_devices(&ids, None, |dev| read_device_synced_groups(paths, dev)),
        |g: &SyncedGroup| g.id.clone(),
        |g: &SyncedGroup| g.updated_at.as_str(),
    );
    // User-ordered by position (old files without the field default to MAX →
    // sort last); name breaks ties for determinism on missing positions.
    merged.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    merged
}

/// Write THIS device's synced-groups file (the device only writes its own —
/// never a peer's). Byte-stable via [`synced_doc::write_stable`]; creates the
/// parent dir.
fn write_own_synced_groups(
    paths: &Paths,
    device_id: &str,
    groups: &[SyncedGroup],
) -> AppResult<()> {
    synced_doc::write_stable(
        &groups_json_path(paths, device_id),
        &SyncedGroupsDoc {
            groups: groups.to_vec(),
        },
    )
}

/// Generate a globally-unique synced-group id: `<deviceId>-<8hex>`. The device
/// prefix is the ownership marker (only this device edits the group), so a peer
/// never collides. Hex 段走中性原语 [`crate::model::generate_short_hex_id`]。
fn generate_synced_group_id(device_id: &str) -> String {
    format!("{device_id}-{}", crate::model::generate_short_hex_id())
}

/// The subset of synced groups THIS device owns (id prefix matches device_id).
fn own_synced_groups(paths: &Paths, device_id: &str) -> Vec<SyncedGroup> {
    read_device_synced_groups(paths, device_id)
        .into_iter()
        .filter(|g| is_owned_by(g, device_id))
        .collect()
}

/// True iff `group` was created by `device_id` (its id carries the prefix).
fn is_owned_by(group: &SyncedGroup, device_id: &str) -> bool {
    group.id.strip_prefix(&format!("{device_id}-")).is_some()
}

/// Create a synced group owned by this device and commit + push (Synced only).
pub fn create_synced_group_owned(
    paths: &Paths,
    cfg: &ConfigData,
    name: &str,
) -> AppResult<SyncedGroup> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Config("group name must not be empty".into()));
    }
    let id = generate_synced_group_id(&cfg.device_id);
    let mut groups = own_synced_groups(paths, &cfg.device_id);
    // New groups append at the END of the current merged display order.
    // Legacy groups (position u32::MAX — files that predate drag-reorder) are
    // excluded so they keep sorting last until the user reorders once.
    let position = read_all_synced_groups(paths)
        .iter()
        .map(|g| g.position)
        .filter(|p| *p != u32::MAX)
        .max()
        .map_or(0, |m| m + 1);
    let group = SyncedGroup {
        id,
        name: name.to_string(),
        device_id: cfg.device_id.clone(),
        updated_at: crate::time::now_iso(),
        position,
    };
    groups.push(group.clone());
    write_own_synced_groups(paths, &cfg.device_id, &groups)?;
    crate::sync::commit_and_push_best_effort(paths, cfg, GROUPS_SYNC_MSG);
    Ok(group)
}

/// Rename a synced group OWNED by this device. A peer's group is read-only here
/// (its owning device will publish the rename on its own round).
pub fn rename_synced_group_owned(
    paths: &Paths,
    cfg: &ConfigData,
    id: &str,
    name: &str,
) -> AppResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Config("group name must not be empty".into()));
    }
    let mut groups = own_synced_groups(paths, &cfg.device_id);
    let g = groups.iter_mut().find(|g| g.id == id).ok_or_else(|| {
        AppError::Config(format!("synced group not found (or not owned here): {id}"))
    })?;
    g.name = name.to_string();
    g.updated_at = crate::time::now_iso();
    write_own_synced_groups(paths, &cfg.device_id, &groups)?;
    crate::sync::commit_and_push_best_effort(paths, cfg, GROUPS_SYNC_MSG);
    Ok(())
}

/// Reorder the synced groups OWNED by this device. `ordered_ids` is the
/// track's FULL displayed order (every synced group, including peers') as
/// submitted by the frontend after a drag; each owned group's `position`
/// becomes its index in that list, so the merged (position, name, id) sort
/// places the drag exactly where it landed — including between peer groups.
/// Peer groups are never written (their positions live in their owners'
/// files) and stale ids are ignored, so a peer group deleted or reordered
/// between fetch and drop cannot fail this write. A drag that changed
/// nothing writes nothing (no empty git commit).
pub fn reorder_synced_groups_owned(
    paths: &Paths,
    cfg: &ConfigData,
    ordered_ids: &[String],
) -> AppResult<()> {
    let mut groups = own_synced_groups(paths, &cfg.device_id);
    if groups.is_empty() {
        return Ok(());
    }
    let mut changed = false;
    for g in &mut groups {
        if let Some(pos) = ordered_ids.iter().position(|id| id == &g.id) {
            if g.position != pos as u32 {
                g.position = pos as u32;
                g.updated_at = crate::time::now_iso();
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(());
    }
    write_own_synced_groups(paths, &cfg.device_id, &groups)?;
    crate::sync::commit_and_push_best_effort(paths, cfg, GROUPS_SYNC_MSG);
    Ok(())
}

/// Delete a synced group OWNED by this device.
pub fn delete_synced_group_owned(paths: &Paths, cfg: &ConfigData, id: &str) -> AppResult<()> {
    let mut groups = own_synced_groups(paths, &cfg.device_id);
    let before = groups.len();
    groups.retain(|g| g.id != id);
    if groups.len() == before {
        return Err(AppError::Config(format!(
            "synced group not found (or not owned here): {id}"
        )));
    }
    write_own_synced_groups(paths, &cfg.device_id, &groups)?;
    crate::sync::commit_and_push_best_effort(paths, cfg, GROUPS_SYNC_MSG);
    Ok(())
}

/// Build the unified `SessionGroup` DTO list (local + synced tracks).
pub(crate) fn list_groups_dto(
    store: &crate::db::Store,
    paths: &Paths,
) -> AppResult<Vec<SessionGroup>> {
    let mut out = Vec::new();
    for lg in store.list_local_groups()? {
        out.push(SessionGroup {
            id: lg.id,
            name: lg.name,
            kind: "local".to_string(),
            device_id: String::new(),
        });
    }
    for sg in read_all_synced_groups(paths) {
        out.push(SessionGroup {
            id: sg.id,
            name: sg.name,
            kind: "synced".to_string(),
            device_id: sg.device_id,
        });
    }
    Ok(out)
}

/// Local group id: 8 hex chars. Device-private, so no prefix is needed (unlike
/// synced groups, which carry a device prefix for cross-device uniqueness).
/// 生成走 model 层的中性原语 [`crate::model::generate_short_hex_id`]（与用户
/// 自建供应商 id 同一设备本地 id 空间）——原语不再住本模块，避免 model 侧
/// 为词法复用 up-call sessions。命令层继续经此入口取 id。
pub(crate) fn generate_local_group_id() -> String {
    crate::model::generate_short_hex_id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    fn cfg(device_id: &str) -> ConfigData {
        ConfigData {
            device_id: device_id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn synced_group_id_carries_device_prefix() {
        let id = generate_synced_group_id("aabbccddeeff");
        assert!(id.starts_with("aabbccddeeff-"));
        assert_eq!(id.len(), "aabbccddeeff-".len() + 8);
    }

    #[test]
    fn read_all_synced_groups_merges_by_id_latest_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Device A owns one group.
        let a = SyncedGroup {
            id: "aabbccddeeff-11111111".into(),
            name: "A-group".into(),
            device_id: "aabbccddeeff".into(),
            updated_at: "2026-08-01T10:00:00.000Z".into(),
            position: 0,
        };
        write_own_synced_groups(&paths, "aabbccddeeff", std::slice::from_ref(&a)).unwrap();
        // Device B owns another.
        let b = SyncedGroup {
            id: "112233445566-22222222".into(),
            name: "B-group".into(),
            device_id: "112233445566".into(),
            updated_at: "2026-08-02T10:00:00.000Z".into(),
            position: 0,
        };
        write_own_synced_groups(&paths, "112233445566", std::slice::from_ref(&b)).unwrap();

        let all = read_all_synced_groups(&paths);
        assert_eq!(all.len(), 2, "both devices' groups merge");
        assert!(all.iter().any(|g| g.id == a.id));
        assert!(all.iter().any(|g| g.id == b.id));
    }

    #[test]
    fn create_rename_delete_synced_group_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = cfg("aabbccddeeff");
        let g = create_synced_group_owned(&paths, &cfg, "Work").unwrap();
        assert!(is_owned_by(&g, "aabbccddeeff"));
        assert_eq!(g.name, "Work");

        rename_synced_group_owned(&paths, &cfg, &g.id, "Work Important").unwrap();
        let all = read_all_synced_groups(&paths);
        assert_eq!(all[0].name, "Work Important");

        delete_synced_group_owned(&paths, &cfg, &g.id).unwrap();
        assert!(read_all_synced_groups(&paths).is_empty());
    }

    #[test]
    fn rename_peer_owned_group_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Seed a peer-owned group under the peer's dir.
        write_own_synced_groups(
            &paths,
            "112233445566",
            &[SyncedGroup {
                id: "112233445566-99999999".into(),
                name: "Peer".into(),
                device_id: "112233445566".into(),
                updated_at: "2026-08-01T00:00:00.000Z".into(),
                position: 0,
            }],
        )
        .unwrap();
        let cfg = cfg("aabbccddeeff");
        // This device does NOT own the group ⇒ reject.
        let err = rename_synced_group_owned(&paths, &cfg, "112233445566-99999999", "x");
        assert!(err.is_err(), "cannot rename a peer's group from here");
    }

    #[test]
    fn read_all_synced_groups_ignores_non_device_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        std::fs::create_dir_all(paths.repo_data.join("not-a-device")).unwrap();
        std::fs::write(
            paths.repo_data.join("not-a-device").join("groups.json"),
            "{\"groups\":[]}",
        )
        .unwrap();
        assert!(read_all_synced_groups(&paths).is_empty());
    }

    #[test]
    fn reorder_synced_groups_applies_full_own_order() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = cfg("aabbccddeeff");
        let a = create_synced_group_owned(&paths, &cfg, "Alpha").unwrap();
        let b = create_synced_group_owned(&paths, &cfg, "Beta").unwrap();
        let c = create_synced_group_owned(&paths, &cfg, "Gamma").unwrap();

        // Reverse the creation order.
        reorder_synced_groups_owned(&paths, &cfg, &[c.id.clone(), b.id.clone(), a.id.clone()])
            .unwrap();

        let ids: Vec<String> = read_all_synced_groups(&paths)
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, [c.id, b.id, a.id]);
    }

    /// Reordering takes the track's FULL displayed order: owned groups get
    /// their position from their index in the list (so a drag can land a
    /// group between peer groups), peer groups are never written.
    #[test]
    fn reorder_renumbers_owned_groups_only() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = cfg("aabbccddeeff");
        let a1 = create_synced_group_owned(&paths, &cfg, "A1").unwrap();
        let a2 = create_synced_group_owned(&paths, &cfg, "A2").unwrap();
        // Peer device B owns b1.
        let peer = SyncedGroup {
            id: "112233445566-99999999".into(),
            name: "B1".into(),
            device_id: "112233445566".into(),
            updated_at: "2026-08-01T00:00:00.000Z".into(),
            position: 0,
        };
        write_own_synced_groups(&paths, "112233445566", std::slice::from_ref(&peer)).unwrap();

        // Drag A2 between the peer group and A1 → full displayed order.
        reorder_synced_groups_owned(&paths, &cfg, &[a1.id.clone(), peer.id, a2.id.clone()])
            .unwrap();

        let own = read_device_synced_groups(&paths, "aabbccddeeff");
        let pos = |id: &str| own.iter().find(|g| g.id == id).unwrap().position;
        assert_eq!(pos(&a1.id), 0);
        assert_eq!(pos(&a2.id), 2, "index in the FULL list, not renumbered");

        // The peer's file is untouched (positions live in the owner's file).
        let peer_after = read_device_synced_groups(&paths, "112233445566");
        assert_eq!(peer_after.len(), 1);
        assert_eq!(peer_after[0].position, 0);
    }

    /// New groups append at the END of the merged display order, even after
    /// a reorder (the user's custom order is not reset by a create).
    #[test]
    fn create_appends_after_reordered_merged_max() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = cfg("aabbccddeeff");
        let a = create_synced_group_owned(&paths, &cfg, "Alpha").unwrap();
        let b = create_synced_group_owned(&paths, &cfg, "Beta").unwrap();
        reorder_synced_groups_owned(&paths, &cfg, &[b.id.clone(), a.id.clone()]).unwrap();

        let c = create_synced_group_owned(&paths, &cfg, "Gamma").unwrap();
        let ids: Vec<String> = read_all_synced_groups(&paths)
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, [b.id, a.id, c.id]);
    }

    /// A reorder that changes nothing must not rewrite the file (no empty
    /// git commit on every drag that lands back where it started).
    #[test]
    fn noop_reorder_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = cfg("aabbccddeeff");
        let a = create_synced_group_owned(&paths, &cfg, "Alpha").unwrap();
        let b = create_synced_group_owned(&paths, &cfg, "Beta").unwrap();

        let path = groups_json_path(&paths, "aabbccddeeff");
        let before = std::fs::read_to_string(&path).unwrap();
        // Same order as stored → no change.
        reorder_synced_groups_owned(&paths, &cfg, &[a.id, b.id]).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(before, after, "noop reorder must not touch the file");
    }

    /// A groups.json written before the `position` field shipped must still
    /// parse; the missing field falls back to MAX so legacy groups sort AFTER
    /// user-ordered ones (never jump to the front).
    #[test]
    fn legacy_groups_json_without_position_sorts_last() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // Hand-write a legacy file: no position field on either group.
        std::fs::create_dir_all(paths.device_data_dir("aabbccddeeff")).unwrap();
        std::fs::write(
            paths
                .device_data_dir("aabbccddeeff")
                .join("groups.json"),
            r#"{"groups":[
                {"id":"aabbccddeeff-11111111","name":"Old","device_id":"aabbccddeeff","updated_at":"2026-08-01T00:00:00.000Z"}
            ]}"#,
        )
        .unwrap();

        let all = read_all_synced_groups(&paths);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].position, u32::MAX, "missing field defaults to MAX");

        // A newer user-ordered group sorts before the legacy one.
        let cfg = cfg("112233445566");
        let fresh = create_synced_group_owned(&paths, &cfg, "Fresh").unwrap();
        let ids: Vec<String> = read_all_synced_groups(&paths)
            .into_iter()
            .map(|g| g.id)
            .collect();
        assert_eq!(ids, [fresh.id, "aabbccddeeff-11111111".to_string()]);
    }

    /// Golden wire bytes: the exact file `write_own_synced_groups` lands,
    /// pinned line-for-line so the shared byte-stable write
    /// ([`crate::synced_doc::stable_bytes`]) can never drift the groups wire
    /// format (pretty JSON + exactly one trailing newline — an unchanged group
    /// list must rewrite identical bytes so pushes stay git no-ops).
    #[test]
    fn write_own_synced_groups_lands_pinned_wire_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        write_own_synced_groups(
            &paths,
            "aabbccddeeff",
            &[SyncedGroup {
                id: "aabbccddeeff-11111111".into(),
                name: "Work".into(),
                device_id: "aabbccddeeff".into(),
                updated_at: "2026-08-01T10:00:00.000Z".into(),
                position: 3,
            }],
        )
        .unwrap();

        let text = std::fs::read_to_string(groups_json_path(&paths, "aabbccddeeff")).unwrap();
        let expected = [
            "{",
            "  \"groups\": [",
            "    {",
            "      \"id\": \"aabbccddeeff-11111111\",",
            "      \"name\": \"Work\",",
            "      \"device_id\": \"aabbccddeeff\",",
            "      \"updated_at\": \"2026-08-01T10:00:00.000Z\",",
            "      \"position\": 3",
            "    }",
            "  ]",
            "}",
        ];
        assert_eq!(
            text.lines().collect::<Vec<&str>>(),
            expected,
            "groups wire bytes drifted"
        );
        assert!(text.ends_with("}\n"), "exactly one trailing newline");
    }
}
