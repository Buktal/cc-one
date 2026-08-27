//! synced_doc — the mechanism layer for "one synced JSON doc per device" (the
//! device-registry write pattern): each device writes ONLY its own file inside
//! the git-synced repo, and readers merge every device's file.
//!
//! What lives here — the mechanism every synced doc shares:
//! - **Tolerant read** ([`read_json_doc`]): a missing / unreadable / unparseable
//!   doc yields `None`. One corrupt or absent peer file must never abort a
//!   whole pull; the caller decides what an absent doc means (usually an empty
//!   item list).
//! - **Schema version gate** ([`schema_ahead_of_build`]): a doc whose `v` is
//!   newer than this binary supports is skipped WHOLE — this build cannot
//!   attribute its content, so merging it under the current keys would
//!   silently mis-merge; its items arrive once the user upgrades. The gate is
//!   optional per domain: a doc type without a `v` field simply never calls
//!   it. Keeping the gate here means the next domain that grows a schema
//!   change gets it instead of hand-rolling a divergent copy.
//! - **Byte-stable write** ([`stable_bytes`] / [`write_stable`]): pretty JSON
//!   plus exactly one trailing newline. Same doc ⇒ same bytes, so an unchanged
//!   store rewrites an identical file and `commit_and_push` stays a no-op —
//!   the push side's git diff depends on it. THE SHAPE IS WIRE FORMAT: the
//!   2-space pretty indent, field-declaration order and the trailing `\n` are
//!   pinned by tests and must not drift.
//! - **latest-wins merge** ([`merge_latest_wins`]): group items by a caller
//!   key; an item displaces the incumbent only when strictly newer on
//!   `updated_at`; an exact tie keeps the first-seen copy.
//! - **Per-device fan-out read** ([`read_all_devices`]): read each device's
//!   doc in `device_ids` order, optionally skipping self's directory.
//!
//! What stays in each domain — true differences, not duplication:
//! - **The doc type itself.** The wrapper struct IS the wire format (its field
//!   names and declaration order serialize into the file), so each domain
//!   declares its own (`SyncedProvidersDoc`, `SyncedGroupsDoc`, …).
//! - **Merge key and display sort.** Providers dedupe on `(app, id)`, synced
//!   groups on `id`; each re-sorts the merge output by its own display order.
//! - **Skip-self or not.** Store-backed domains (providers, session snapshots)
//!   pass `Some(self_id)` — self is local-authoritative, so a pull must never
//!   read back a possibly-stale git copy of this device's own file. Synced
//!   groups pass `None` — their file IS the authoritative storage (no DB
//!   copy), so a device reads its own file back like a peer's.
//! - **Carrying a schema `v` or not.** Providers do (their item shape has
//!   grown app pools / live-managed flags). Synced groups do not yet: one
//!   shape so far, and writing a constant `v` would gate nothing while
//!   rewriting every device's file.
//!
//! Placement: a crate-level module next to `devices.rs`. It is cross-domain
//! mechanism consumed by `provider::sync`, `sessions` and `devices` — it does
//! not belong to any one of them, and it is not git knowledge (so not inside
//! `sync/`, the libgit2 orchestration layer). It depends on nothing
//! domain-shaped: plain paths and device-id lists in, generic docs and items
//! out (the device-id walk itself stays in `devices::iter_data_device_ids`).

use std::path::Path;

use crate::error::AppResult;

/// Tolerant read of one synced doc: missing / unreadable / unparseable ⇒
/// `None`. One corrupt peer file must never abort a pull.
pub(crate) fn read_json_doc<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Read-side schema version gate: `true` ⇒ the doc's `v` is NEWER than this
/// binary supports and the doc must be skipped whole (a warning is logged —
/// the items simply arrive after the user upgrades). A doc without a `v`
/// reads as 0 and never trips. `what` names the file in the log (e.g.
/// "provider file for device \<id\>").
pub(crate) fn schema_ahead_of_build(v: u32, max_supported: u32, what: &str) -> bool {
    if v <= max_supported {
        return false;
    }
    eprintln!(
        "[cc-one] {what} is schema v{v} (this build reads ≤ v{max_supported}) — skipped whole; upgrade to read it"
    );
    true
}

/// Serialize a doc to its byte-stable file content: pretty JSON plus exactly
/// one trailing newline (2-space indent, fields in declaration order). Same
/// doc ⇒ same bytes, so an unchanged store rewrites an identical file and the
/// push stays a git no-op.
pub(crate) fn stable_bytes<T: serde::Serialize>(doc: &T) -> AppResult<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(doc)?))
}

/// Byte-stable write ([`stable_bytes`]) with parent-dir creation — the
/// push-side writer for a device's own doc file.
pub(crate) fn write_stable<T: serde::Serialize>(path: &Path, doc: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, stable_bytes(doc)?)?;
    Ok(())
}

/// latest-wins merge: items grouped by `key_of`; an item displaces the
/// incumbent only when its `updated_at` is strictly newer, so an exact tie
/// keeps the first-seen copy (input order decides). The merge key and the
/// display sort applied to the result are the caller's domain rules; the
/// output order here is unspecified (map order) — every caller re-sorts.
pub(crate) fn merge_latest_wins<T, K: Eq + std::hash::Hash>(
    items: impl IntoIterator<Item = T>,
    key_of: impl Fn(&T) -> K,
    updated_at_of: impl Fn(&T) -> &str,
) -> Vec<T> {
    let mut by_key: std::collections::HashMap<K, T> = std::collections::HashMap::new();
    for item in items {
        let key = key_of(&item);
        let take = by_key
            .get(&key)
            .map(|e| updated_at_of(e) < updated_at_of(&item))
            .unwrap_or(true);
        if take {
            by_key.insert(key, item);
        }
    }
    by_key.into_values().collect()
}

/// Read every device's doc via `read_one`, in `device_ids` order (the caller
/// resolved them — usually `devices::iter_data_device_ids`), optionally
/// skipping self's directory: `skip_self = Some(self_id)` marks the
/// store-backed domains (self is local-authoritative; a possibly-stale git
/// copy of this device's file must never be read back), `None` marks the
/// file-as-authoritative domains (the device's own file is the storage and is
/// read back like a peer's).
pub(crate) fn read_all_devices<T>(
    device_ids: &[String],
    skip_self: Option<&str>,
    mut read_one: impl FnMut(&str) -> Vec<T>,
) -> Vec<T> {
    let mut out = Vec::new();
    for id in device_ids {
        if Some(id.as_str()) == skip_self {
            continue;
        }
        out.extend(read_one(id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Probe {
        v: u32,
        name: String,
    }

    /// The byte-stable shape is pinned wire format: pretty JSON, fields in
    /// declaration order, exactly one trailing newline. Drifting this
    /// rewrites every device's next push (git churn across the fleet).
    #[test]
    fn stable_bytes_is_pretty_json_plus_one_trailing_newline() {
        assert_eq!(
            stable_bytes(&Probe {
                v: 3,
                name: "x".into()
            })
            .unwrap(),
            "{\n  \"v\": 3,\n  \"name\": \"x\"\n}\n"
        );
    }

    #[test]
    fn write_stable_lands_exact_bytes_and_creates_parents() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/dir/doc.json");
        write_stable(
            &path,
            &Probe {
                v: 1,
                name: "y".into(),
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"v\": 1,\n  \"name\": \"y\"\n}\n"
        );
    }

    #[test]
    fn read_json_doc_tolerates_missing_and_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("d.json");
        assert_eq!(read_json_doc::<Probe>(&path), None, "missing ⇒ None");
        std::fs::write(&path, "{oops").unwrap();
        assert_eq!(read_json_doc::<Probe>(&path), None, "corrupt ⇒ None");
        std::fs::write(&path, "{\"v\":2,\"name\":\"ok\"}").unwrap();
        let doc = read_json_doc::<Probe>(&path).unwrap();
        assert_eq!(doc.v, 2);
        assert_eq!(doc.name, "ok");
    }

    /// The gate trips only on a STRICTLY newer `v`: absent (0) and equal
    /// versions read, newer ones skip (the caller then drops the doc).
    #[test]
    fn schema_gate_skips_only_strictly_newer() {
        assert!(!schema_ahead_of_build(0, 3, "probe"));
        assert!(!schema_ahead_of_build(3, 3, "probe"));
        assert!(schema_ahead_of_build(4, 3, "probe"));
    }

    #[derive(Debug, PartialEq)]
    struct Row {
        k: &'static str,
        ts: &'static str,
        tag: &'static str,
    }

    /// Newest `updated_at` wins regardless of arrival order; an exact tie
    /// keeps the FIRST-seen copy.
    #[test]
    fn merge_latest_wins_newest_wins_ties_first_seen() {
        let merged = merge_latest_wins(
            [
                Row {
                    k: "a",
                    ts: "1",
                    tag: "old",
                },
                Row {
                    k: "a",
                    ts: "3",
                    tag: "new",
                },
                Row {
                    k: "a",
                    ts: "2",
                    tag: "mid",
                },
                Row {
                    k: "b",
                    ts: "1",
                    tag: "only",
                },
            ],
            |r| r.k,
            |r| r.ts,
        );
        let by_k: std::collections::HashMap<&str, &Row> = merged.iter().map(|r| (r.k, r)).collect();
        assert_eq!(by_k["a"].tag, "new", "strictly newer displaces");
        assert_eq!(by_k["b"].tag, "only");

        let tie = merge_latest_wins(
            [
                Row {
                    k: "a",
                    ts: "1",
                    tag: "first",
                },
                Row {
                    k: "a",
                    ts: "1",
                    tag: "second",
                },
            ],
            |r| r.k,
            |r| r.ts,
        );
        assert_eq!(tie.len(), 1);
        assert_eq!(tie[0].tag, "first", "tie keeps the first-seen copy");
    }

    /// Fan-out walks every device in order, skipping self ONLY when asked —
    /// the explicit store-backed vs file-as-authoritative domain difference.
    #[test]
    fn read_all_devices_fans_out_and_skips_self_only_when_asked() {
        fn seen(id: &str) -> Vec<String> {
            vec![format!("{id}-item")]
        }
        let ids: Vec<String> = vec!["aaa".into(), "bbb".into()];
        assert_eq!(
            read_all_devices(&ids, Some("aaa"), seen),
            vec!["bbb-item".to_string()],
            "store-backed: self's own file is not read back"
        );
        assert_eq!(
            read_all_devices(&ids, None, seen),
            vec!["aaa-item".to_string(), "bbb-item".to_string()],
            "file-as-authoritative: self's own file is read back too"
        );
        assert_eq!(read_all_devices(&ids, Some("zzz"), seen).len(), 2);
    }
}
