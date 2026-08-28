//! Per-day JSONL Artifact — the derived snapshot of the Local Store that git
//! syncs. Two grains share this machinery: per-call `UsageRecord`
//! (`usage-<day>.jsonl`) and per-turn `TurnDuration` (`turns-<day>.jsonl`).
//!
//! collect only writes SQLite (+ marks days dirty); the **push** path rewrites
//! each dirty day's file from the store ([`recompute_usage_day`] /
//! [`recompute_turns_day`]), and **pull** reads peers' files back into the store
//! ([`read_device_artifacts`] / [`read_device_turn_artifacts`], one device dir
//! at a time — the pull-side coarse gate decides WHICH dirs to re-read via
//! [`artifact_dir_sig`]). Only the row type, file-name prefix, and day accessor
//! differ — captured by [`ArtifactGrain`] so the policy lives in one place.

use std::collections::BTreeMap;
use std::path::Path;

#[cfg(test)]
use super::jsonl::write_jsonl_file;
use super::jsonl::{read_jsonl_file_of, rewrite_jsonl_file};
use crate::config::Paths;
use crate::db::Store;
use crate::error::AppResult;
use crate::model::{TurnDuration, UsageRecord};

/// One JSONL Artifact grain: its row type and file-name prefix. The per-day
/// split is driven by the `day` column in SQL (`usage_for_day_device`), not by a
/// trait method, so the trait stays minimal.
trait ArtifactGrain {
    type Row: serde::Serialize + serde::de::DeserializeOwned;
    /// File-name prefix; the Artifact is `<prefix>-<day>.jsonl`.
    const PREFIX: &'static str;
}

/// Per-call usage records → `usage-<day>.jsonl`.
struct UsageGrain;
impl ArtifactGrain for UsageGrain {
    type Row = UsageRecord;
    const PREFIX: &'static str = "usage";
}

/// Per-turn durations → `turns-<day>.jsonl`.
struct TurnGrain;
impl ArtifactGrain for TurnGrain {
    type Row = TurnDuration;
    const PREFIX: &'static str = "turns";
}

/// `<device_data_dir>/<deviceId>/<prefix>-<day>.jsonl`.
fn day_path<A: ArtifactGrain>(paths: &Paths, device_id: &str, day: &str) -> std::path::PathBuf {
    paths
        .device_data_dir(device_id)
        .join(format!("{}-{day}.jsonl", A::PREFIX))
}

/// Recompute one device's per-day usage Artifact from the store: every
/// `usage_records` row for (day, device) in uuid order, as a full file rewrite.
/// The push-side writer — collect never touches the Artifact; the store is the
/// single source of truth and this materializes the derived snapshot a peer
/// pulls. Byte-stable across pushes (uuid order + field declaration order).
/// Returns the row count — the caller (push) uses it as the recompute-time
/// snapshot to decide whether the day is still clearable after the push lands.
pub fn recompute_usage_day(
    store: &Store,
    paths: &Paths,
    device_id: &str,
    day: &str,
) -> AppResult<usize> {
    let rows = store.usage_for_day_device(day, device_id)?;
    rewrite_jsonl_file(&day_path::<UsageGrain>(paths, device_id, day), &rows)?;
    Ok(rows.len())
}

/// Recompute one device's per-day turn-duration Artifact from the store (mirrors
/// [`recompute_usage_day`] for the per-turn grain; same row-count snapshot role).
pub fn recompute_turns_day(
    store: &Store,
    paths: &Paths,
    device_id: &str,
    day: &str,
) -> AppResult<usize> {
    let rows = store.turns_for_day_device(day, device_id)?;
    rewrite_jsonl_file(&day_path::<TurnGrain>(paths, device_id, day), &rows)?;
    Ok(rows.len())
}

/// `<prefix>-*.jsonl` under the device dir?
fn is_artifact_of<A: ArtifactGrain>(path: &Path) -> bool {
    let prefix_dash = format!("{}-", A::PREFIX);
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with(&prefix_dash))
            .unwrap_or(false)
}

/// Read every `<prefix>-*.jsonl` Artifact for one device.
fn read_device_artifacts_of<A: ArtifactGrain>(
    paths: &Paths,
    device_id: &str,
) -> AppResult<Vec<A::Row>> {
    let dir = paths.device_data_dir(device_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let p = entry.path();
        if is_artifact_of::<A>(&p) {
            out.extend(read_jsonl_file_of::<A::Row>(&p)?);
        }
    }
    Ok(out)
}

/// One device's usage Artifacts (production — the pull path imports one
/// device dir at a time, gated by [`artifact_dir_sig`]).
pub(crate) fn read_device_artifacts(paths: &Paths, device_id: &str) -> AppResult<Vec<UsageRecord>> {
    read_device_artifacts_of::<UsageGrain>(paths, device_id)
}

/// One device's turn-duration Artifacts (the `turns-<day>.jsonl` grain of
/// [`read_device_artifacts`]).
pub(crate) fn read_device_turn_artifacts(
    paths: &Paths,
    device_id: &str,
) -> AppResult<Vec<TurnDuration>> {
    read_device_artifacts_of::<TurnGrain>(paths, device_id)
}

/// One device artifact dir's signature — the pull-side coarse gate's state.
/// `filename → (mtime_nanos, len)` for every `<usage|turns>-*.jsonl` file in
/// the dir; empty for an absent dir. Computed from `read_dir` + metadata
/// only — no file content is read, so an unchanged dir costs a directory
/// listing, not a JSONL re-parse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ArtifactDirSig {
    files: BTreeMap<String, (u128, u64)>,
}

/// Compute a device dir's [`ArtifactDirSig`]. An absent dir yields the empty
/// signature — caching it makes a LATER-appearing dir (a peer's first push)
/// read on the next pull, because its non-empty signature differs.
///
/// The gate is deliberately coarse (mtime + length, no content hash): a file
/// rewritten within the filesystem's mtime granularity with an unchanged
/// length could be missed until its next rewrite. Under the byte-stable
/// rewrite contract that means identical bytes (harmless); a real content
/// change always rewrites the file, refreshing its mtime.
pub(crate) fn artifact_dir_sig(paths: &Paths, device_id: &str) -> AppResult<ArtifactDirSig> {
    let dir = paths.device_data_dir(device_id);
    if !dir.exists() {
        return Ok(ArtifactDirSig::default());
    }
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if !(is_artifact_of::<UsageGrain>(&path) || is_artifact_of::<TurnGrain>(&path)) {
            continue;
        }
        let meta = entry.metadata()?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        files.insert(name, (mtime, meta.len()));
    }
    Ok(ArtifactDirSig { files })
}

/// Stand up a device's usage Artifact for the sync round-trip tests — group the
/// records by day and append each day's file idempotently (a row already in the
/// file is skipped). Test fixture only: production writes the Artifact via
/// [`recompute_usage_day`], not append.
#[cfg(test)]
pub(crate) fn append_jsonl(
    paths: &Paths,
    device_id: &str,
    records: &[UsageRecord],
) -> AppResult<()> {
    use std::collections::{BTreeMap, HashSet};
    let mut by_day: BTreeMap<String, Vec<&UsageRecord>> = BTreeMap::new();
    for r in records {
        by_day.entry(r.day.clone()).or_default().push(r);
    }
    for (day, day_rows) in by_day {
        let path = day_path::<UsageGrain>(paths, device_id, &day);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let existing: HashSet<String> = read_jsonl_file_of::<UsageRecord>(&path)
            .unwrap_or_default()
            .into_iter()
            .map(|r| r.uuid)
            .collect();
        let missing: Vec<&UsageRecord> = day_rows
            .into_iter()
            .filter(|r| !existing.contains(&r.uuid))
            .collect();
        if missing.is_empty() {
            continue;
        }
        write_jsonl_file(&path, &missing)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::ingest::ingest_collected;
    use crate::model::{ServerToolUse, TokenCounts};
    use crate::pricing::seed_book;
    use crate::source_parser::{CollectResult, RawTurnDuration, RawUsage};

    fn raw(uuid: &str, model: &str) -> RawUsage {
        RawUsage {
            uuid: uuid.into(),
            timestamp: "2026-07-13T16:55:22.467Z".into(),
            model: model.into(),
            source: "claude_code".into(),
            session_id: String::new(),
            tokens: TokenCounts {
                input: 1000,
                output: 500,
                cache_creation: 0,
                cache_read: 0,
            },
            server_tool_use: ServerToolUse::default(),
            stop_reason: "end_turn".into(),
            service_tier: "standard".into(),
            iterations: 0,
        }
    }

    fn raw_turn(uuid: &str) -> RawTurnDuration {
        RawTurnDuration {
            uuid: uuid.into(),
            timestamp: "2026-07-13T16:55:00Z".into(),
            session_id: String::new(),
            duration_ms: 123_456,
        }
    }

    /// Recompute is byte-stable: the same store yields identical file bytes every
    /// time, and rows land in uuid order (not collect order). This is what keeps
    /// a settled day from churning git across pushes.
    #[test]
    fn recompute_usage_day_is_byte_stable_and_uuid_ordered() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = seed_book();
        let dev = "0123456789ab";
        // Ingest "zzz" before "aaa": collect order is unstable, uuid order fixed.
        let result = CollectResult {
            source: "claude_code".into(),
            events: vec![raw("zzz", "glm-5.2"), raw("aaa", "glm-5.2")],
            corrections: vec![],
            turn_durations: vec![],
            files_scanned: 1,
            lines_skipped: 0,
            sessions: vec![],
            messages: vec![],
            session_ids: vec![],
        };
        ingest_collected(&store, &paths, dev, &book, result).unwrap();
        let day_file = paths.device_data_dir(dev).join("usage-2026-07-13.jsonl");

        recompute_usage_day(&store, &paths, dev, "2026-07-13").unwrap();
        let bytes1 = std::fs::read(&day_file).unwrap();
        let text = String::from_utf8(bytes1.clone()).unwrap();
        assert!(
            text.find("\"aaa\"").unwrap() < text.find("\"zzz\"").unwrap(),
            "rows emitted in uuid order, not collect order"
        );

        // Recompute again ⇒ identical bytes (idempotent / byte-stable).
        recompute_usage_day(&store, &paths, dev, "2026-07-13").unwrap();
        let bytes2 = std::fs::read(&day_file).unwrap();
        assert_eq!(bytes1, bytes2, "recompute is byte-stable across calls");
    }

    /// collect leaves the Artifact unwritten; recompute materializes the day's
    /// full content from the store (the push step). Also covers gap self-heal: a
    /// row in the store but absent from the file is filled by recompute.
    #[test]
    fn recompute_materializes_the_day_collect_left_unwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = seed_book();
        let dev = "0123456789ab";
        let result = CollectResult {
            source: "claude_code".into(),
            events: vec![raw("a", "glm-5.2")],
            corrections: vec![],
            turn_durations: vec![],
            files_scanned: 1,
            lines_skipped: 0,
            sessions: vec![],
            messages: vec![],
            session_ids: vec![],
        };
        ingest_collected(&store, &paths, dev, &book, result).unwrap();
        let day_file = paths.device_data_dir(dev).join("usage-2026-07-13.jsonl");
        assert!(!day_file.exists(), "collect does not write the Artifact");
        recompute_usage_day(&store, &paths, dev, "2026-07-13").unwrap();
        let read = read_device_artifacts_of::<UsageGrain>(&paths, dev).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].uuid, "a");
    }

    /// usage and turns are separate grains/files; recomputing a day writes each,
    /// each holding only its own grain (usage read never picks up turns, etc.).
    #[test]
    fn recompute_keeps_usage_and_turn_grains_separate() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let book = seed_book();
        let dev = "0123456789ab";
        let result = CollectResult {
            source: "claude_code".into(),
            events: vec![raw("a", "glm-5.2")],
            corrections: vec![],
            turn_durations: vec![raw_turn("td1"), raw_turn("td2")],
            files_scanned: 1,
            lines_skipped: 0,
            sessions: vec![],
            messages: vec![],
            session_ids: vec![],
        };
        ingest_collected(&store, &paths, dev, &book, result).unwrap();
        recompute_usage_day(&store, &paths, dev, "2026-07-13").unwrap();
        recompute_turns_day(&store, &paths, dev, "2026-07-13").unwrap();
        let usage = read_device_artifacts_of::<UsageGrain>(&paths, dev).unwrap();
        let turns = read_device_artifacts_of::<TurnGrain>(&paths, dev).unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(turns.len(), 2);
    }

    /// A day with no store rows for the device ⇒ recompute removes any stale file
    /// rather than leaving an empty Artifact behind.
    #[test]
    fn recompute_drops_a_day_file_with_no_rows() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let dev = "0123456789ab";
        let day_file = paths.device_data_dir(dev).join("usage-2026-07-13.jsonl");
        std::fs::create_dir_all(day_file.parent().unwrap()).unwrap();
        std::fs::write(&day_file, "stale\n").unwrap();
        // No rows in the store for this day/device ⇒ recompute clears the file.
        recompute_usage_day(&store, &paths, dev, "2026-07-13").unwrap();
        assert!(!day_file.exists(), "empty day ⇒ stale file removed");
    }

    /// The pull-side coarse gate's signature: an absent dir is empty; files
    /// land under their name with mtime+len; a rewrite (even byte-identical —
    /// push rewrites the file, refreshing its mtime) changes it; an unchanged
    /// dir yields the same signature again (the gate's skip condition).
    #[test]
    fn artifact_dir_sig_tracks_names_mtime_and_len() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let dev = "0123456789ab";
        assert_eq!(
            artifact_dir_sig(&paths, dev).unwrap(),
            ArtifactDirSig::default(),
            "absent dir ⇒ empty signature"
        );

        let dir = paths.device_data_dir(dev);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("usage-2026-07-13.jsonl");
        std::fs::write(&file, "{\"uuid\":\"u1\"}\n").unwrap();
        let s1 = artifact_dir_sig(&paths, dev).unwrap();
        assert_eq!(s1.files.len(), 1, "only artifact files are tracked");
        assert_eq!(s1.files["usage-2026-07-13.jsonl"].1, 14, "length captured");

        // A byte-identical rewrite (what a settled day's recompute does) still
        // bumps the mtime component — the gate re-reads, the PK dedups.
        std::fs::write(&file, "{\"uuid\":\"u1\"}\n").unwrap();
        let s2 = artifact_dir_sig(&paths, dev).unwrap();
        assert_ne!(s1, s2, "rewrite refreshes the mtime component");

        // Unchanged dir ⇒ identical signature (the gate skips the re-read).
        let s3 = artifact_dir_sig(&paths, dev).unwrap();
        assert_eq!(s2, s3, "unchanged dir ⇒ stable signature");
    }
}
