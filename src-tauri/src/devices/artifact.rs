//! Device-name artifact（`repo/config/devices_<id>.json`，一设备一文件）：
//! 本机发布自身身份、读取对端身份的云端 registry 载体，由普通 Git sync 携带。
//! 读侧兼容旧的 `config/devices/<id>.json` 布局（只读回退，新布局优先）。

use std::collections::HashSet;

use crate::config::Paths;
use crate::error::AppResult;
use crate::model::DeviceArtifact;
use crate::synced_doc;

use super::id::is_valid_device_id;

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
    if !is_valid_device_id(id) {
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
