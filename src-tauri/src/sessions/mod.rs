//! Session management — the domain logic behind the sessions Tauri commands
//! (the command layer itself lives in `commands`).
//!
//! Two group tracks:
//! - **Local** (`local_groups` SQLite table): device-private, CRUD immediate,
//!   never in git. Owned by `db::Store`.
//! - **Synced** (`data/<deviceId>/groups.json`): cross-device via git. Each
//!   device writes ONLY its own file; reading merges every device's file by id
//!   (the device-registry pattern). Ids carry a device prefix
//!   (`<deviceId>-<8hex>`) so they are globally unique without coordination.
//!
//! Session CRUD (favorited / custom_title / group membership / list / transcript
//! read) is layered over `db::Store` (sessions table) + `collect::ingest` (transcript
//! I/O). The `commands` module's write commands call the `*_owned` operations
//! here and emit `"sessions_changed"` so the frontend refreshes its queries.

pub mod session_snapshot;
pub mod snapshot_policy;

use std::path::PathBuf;

use crate::config::{ConfigData, Paths};
use crate::error::{AppError, AppResult};
use crate::model::{SessionGroup, SyncedGroup};

/// Per-device synced-groups file: `repo/data/<deviceId>/groups.json`.
fn groups_json_path(paths: &Paths, device_id: &str) -> PathBuf {
    paths.device_data_dir(device_id).join("groups.json")
}

/// Wrapper so the file is a stable JSON object with one array (extensible
/// without a wire break later). Missing file ⇒ empty doc.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct SyncedGroupsDoc {
    #[serde(default)]
    groups: Vec<SyncedGroup>,
}

/// Read one device's synced-groups file. Missing/unreadable ⇒ empty.
fn read_device_synced_groups(paths: &Paths, device_id: &str) -> Vec<SyncedGroup> {
    let path = groups_json_path(paths, device_id);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<SyncedGroupsDoc>(&text)
        .unwrap_or_default()
        .groups
}

/// Every device's synced groups merged by id (latest `updated_at` wins; ties →
/// first-seen). Iterates only valid device dirs so a stray folder never shows
/// up as a groups source. This is the read-side of the per-device-write pattern
/// (mirrors `devices::read_all_device_artifacts`).
pub fn read_all_synced_groups(paths: &Paths) -> Vec<SyncedGroup> {
    let mut by_id: std::collections::HashMap<String, SyncedGroup> =
        std::collections::HashMap::new();
    for name in crate::devices::iter_data_device_ids(paths).unwrap_or_default() {
        for g in read_device_synced_groups(paths, &name) {
            let existing = by_id.get(&g.id);
            let take = existing
                .map(|e| e.updated_at < g.updated_at)
                .unwrap_or(true);
            if take {
                by_id.insert(g.id.clone(), g);
            }
        }
    }
    let mut out: Vec<SyncedGroup> = by_id.into_values().collect();
    // User-ordered by position (old files without the field default to MAX →
    // sort last); name breaks ties for determinism on missing positions.
    out.sort_by(|a, b| {
        a.position
            .cmp(&b.position)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    out
}

/// Write THIS device's synced-groups file (the device only writes its own —
/// never a peer's). Creates the parent dir.
fn write_own_synced_groups(
    paths: &Paths,
    device_id: &str,
    groups: &[SyncedGroup],
) -> AppResult<()> {
    let path = groups_json_path(paths, device_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let doc = SyncedGroupsDoc {
        groups: groups.to_vec(),
    };
    let json = serde_json::to_string_pretty(&doc)?;
    std::fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

/// Generate a globally-unique synced-group id: `<deviceId>-<8hex>`. The device
/// prefix is the ownership marker (only this device edits the group), so a peer
/// never collides.
fn generate_synced_group_id(device_id: &str) -> String {
    use rand::Rng;
    let bytes: [u8; 4] = rand::thread_rng().gen();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{device_id}-{hex}")
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
    crate::sync::commit_and_push_best_effort(paths, cfg, "cc-one: groups sync");
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
    crate::sync::commit_and_push_best_effort(paths, cfg, "cc-one: groups sync");
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
    crate::sync::commit_and_push_best_effort(paths, cfg, "cc-one: groups sync");
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
    crate::sync::commit_and_push_best_effort(paths, cfg, "cc-one: groups sync");
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
pub(crate) fn generate_local_group_id() -> String {
    use rand::Rng;
    let bytes: [u8; 4] = rand::thread_rng().gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
}
