//! Library — a per-device, git-mediated cloud-storage relay.
//!
//! Users drop arbitrary files / dirs in; they land under
//! `repo/library/<deviceId>/` and ride the normal Git sync. Upload is the only
//! automatic direction (drag ⇒ write + push). Download is manual — the user
//! exports an item to a path they choose; cc one never writes into an AI
//! tool's own config dir. Same-name same-kind overwrites (Git history is the
//! safety net); same-name different-kind is rejected (a path cannot be both a
//! file and a directory, and the delete-then-create it would need is
//! destructive). Per-device subtrees never collide across devices.
//!
//! 前端传入的路径参数（subpath / rel_path / device_scope / peer_id）一律经
//! [`device_subdir`] 的包含性谓词定位：写盘只发生在 library root 内。
//! 谓词本身在 [`paths`]，对端遗忘的 library 副作用在 [`forget`]。

use std::path::{Path, PathBuf};

use crate::config::{ConfigData, ConfigStore};
use crate::db::Store;
use crate::error::{AppError, AppResult};

mod forget;
mod paths;
#[cfg(test)]
mod tests;

pub(crate) use forget::count_subtree;
pub use forget::forget_device_library;
pub(crate) use paths::has_only_plain_components;
use paths::{device_subdir, is_plain_entry_name};

/// A Library entry is either a single file or a directory tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    File,
    Dir,
}

/// One entry under a device's Library subtree, as shown in the list.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct LibraryEntry {
    /// Display name (file or dir basename).
    pub name: String,
    pub kind: LibraryKind,
    /// Bytes (files only; 0 for dirs — size is not recursed).
    pub size: f64,
    /// Epoch millis (f64 — specta-safe, dayjs-friendly).
    pub modified_ms: f64,
    /// Owning device id.
    pub device_id: String,
    /// Owning device display name (self name or a known alias).
    pub device_name: String,
    pub is_self: bool,
    /// Path relative to the library root: `<deviceId>/<sub...>/<name>`. Used to
    /// target delete / rename / export.
    pub rel_path: String,
    /// Absolute filesystem path, for the frontend's `convertFileSrc` preview.
    pub abs_path: String,
}

/// One item the user is uploading (from the pending-upload dialog).
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
pub struct UploadItem {
    /// Absolute source path on this machine (from the drag-drop event).
    pub source_path: String,
    /// Final name in the library (the user may have renamed it).
    pub target_name: String,
}

/// What `forget_device` does with a peer's library subtree
/// (`repo/library/<peerId>/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LibraryForgetAction {
    /// Move the subtree into THIS device's library under `from-<peer>/`.
    Migrate,
    /// Delete the subtree outright.
    Delete,
}

/// File/folder counts for a device's library subtree, shown in the
/// forget-device dialog so the user sees what they are migrating or deleting.
/// f64 to stay specta-safe across the TS boundary (counts, never fractional).
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct DeviceLibrarySummary {
    pub files: f64,
    pub dirs: f64,
}

/// Special device-scope value meaning "every device".
const SCOPE_ALL: &str = "all";

// ---------------------------------------------------------------------------
// scan
// ---------------------------------------------------------------------------

/// List the direct children of `(device_scope, subpath)` under the library
/// root. `device_scope = "all"` aggregates every device dir; a specific id
/// scopes to one. `subpath` is relative to each device's own root (used when
/// drilling into a directory). `is_self` and the device's display name are
/// layered on from the config.
pub fn scan(
    store: &Store,
    config: &ConfigStore,
    device_scope: &str,
    subpath: &str,
) -> AppResult<Vec<LibraryEntry>> {
    let paths = config.paths();
    let cfg = config.get();

    // Device enumeration + display names both come from the device registry —
    // the single source — so the library view shows the same device set and the
    // same names a dashboard list would (a peer's published name, not a raw id).
    let device_ids = match device_scope {
        SCOPE_ALL | "" => crate::devices::known_device_ids(&paths, &cfg),
        id => vec![id.to_string()],
    };
    let device_names = crate::devices::resolve_display_names(store, &cfg, &device_ids)?;

    let mut out = Vec::new();
    for did in device_ids {
        let dir = device_subdir(&paths, &did, subpath)?;
        if !dir.is_dir() {
            continue;
        }
        let is_self = crate::devices::is_self(&cfg, &did);
        let device_name = device_names
            .get(&did)
            .cloned()
            .unwrap_or_else(|| did.clone());
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".gitkeep" {
                continue;
            }
            let meta = entry.metadata()?;
            let kind = if meta.is_dir() {
                LibraryKind::Dir
            } else {
                LibraryKind::File
            };
            let size = if meta.is_file() {
                meta.len() as f64
            } else {
                0.0
            };
            let modified_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as f64)
                .unwrap_or(0.0);
            let rel_path = join_rel(&did, subpath, &name);
            let abs_path = entry.path().to_string_lossy().to_string();
            out.push(LibraryEntry {
                name,
                kind,
                size,
                modified_ms,
                device_id: did.clone(),
                device_name: device_name.clone(),
                is_self,
                rel_path,
                abs_path,
            });
        }
    }
    Ok(out)
}

/// `<deviceId>/<sub>/<name>`, forward-slash joined for cross-platform rel paths.
fn join_rel(device_id: &str, subpath: &str, name: &str) -> String {
    let sub = subpath.trim().trim_matches('/');
    if sub.is_empty() {
        format!("{device_id}/{name}")
    } else {
        format!("{device_id}/{sub}/{name}")
    }
}

// ---------------------------------------------------------------------------
// upload
// ---------------------------------------------------------------------------

/// Copy each pending item into this device's library subtree at `subpath`,
/// overwriting same-name same-kind entries. Rejects same-name different-kind.
/// On success commits + pushes the batch (best-effort, Synced only) — the
/// push is part of the change entry, so a caller cannot upload without
/// publishing it.
pub fn upload(
    paths: &crate::config::Paths,
    cfg: &ConfigData,
    items: &[UploadItem],
    subpath: &str,
) -> AppResult<()> {
    if cfg.device_id.is_empty() {
        return Err(AppError::Config("device id not initialized".into()));
    }
    let dest_dir = device_subdir(paths, &cfg.device_id, subpath)?;
    std::fs::create_dir_all(&dest_dir)?;
    for item in items {
        let src = Path::new(&item.source_path);
        if !src.exists() {
            return Err(AppError::Config(format!(
                "source not found: {}",
                src.display()
            )));
        }
        let name = item.target_name.trim();
        if !is_plain_entry_name(name) || name == ".gitkeep" {
            return Err(AppError::Config(format!("invalid target name: {name}")));
        }
        let dst = dest_dir.join(name);
        // Reject same-name different-kind (a path cannot be both file and dir,
        // and the delete-then-create it would need is destructive).
        if dst.exists() {
            match (src.is_dir(), dst.is_dir()) {
                (true, false) => {
                    return Err(AppError::Config(format!(
                        "{name} exists as a file; cannot overwrite with a directory"
                    )));
                }
                (false, true) => {
                    return Err(AppError::Config(format!(
                        "{name} exists as a directory; cannot overwrite with a file"
                    )));
                }
                _ => {}
            }
        }
        // Overwrite same-kind: drop the existing target first.
        if dst.exists() {
            if dst.is_dir() {
                std::fs::remove_dir_all(&dst)?;
            } else {
                std::fs::remove_file(&dst)?;
            }
        }
        if src.is_dir() {
            copy_dir_recursive(src, &dst)?;
            let _ = ensure_gitkeep(&dst);
        } else {
            std::fs::copy(src, &dst)?;
        }
    }
    commit_push_library(paths, cfg);
    Ok(())
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &Path, dst: &Path) -> AppResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Git does not track empty directories — drop a `.gitkeep` so an emptied /
/// newly-empty dir still syncs.
fn ensure_gitkeep(dir: &Path) -> AppResult<()> {
    if std::fs::read_dir(dir)?.next().is_none() {
        std::fs::write(dir.join(".gitkeep"), b"")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// export / delete / rename
// ---------------------------------------------------------------------------

/// Copy a library entry (file or dir) into a target dir the user chose. The
/// entry keeps its name; cc one never writes into an AI tool's own paths.
pub fn export_entry(
    paths: &crate::config::Paths,
    rel_path: &str,
    target_dir: &str,
) -> AppResult<()> {
    let src = resolve_rel(paths, rel_path)?;
    let name = src
        .file_name()
        .ok_or_else(|| AppError::Config("entry has no name".into()))?;
    let dst = Path::new(target_dir).join(name);
    if src.is_dir() {
        copy_dir_recursive(&src, &dst)?;
    } else {
        std::fs::copy(&src, &dst)?;
    }
    Ok(())
}

/// Delete a library entry (file or dir), then commit + push the deletion
/// (best-effort, Synced only) — the push is part of the change entry, so a
/// caller cannot delete without publishing the deletion.
pub fn delete_entry(
    paths: &crate::config::Paths,
    cfg: &ConfigData,
    rel_path: &str,
) -> AppResult<()> {
    let target = resolve_rel(paths, rel_path)?;
    if target.is_dir() {
        std::fs::remove_dir_all(&target)?;
    } else {
        std::fs::remove_file(&target)?;
    }
    // Re-seal the now-maybe-empty parent with .gitkeep.
    if let Some(parent) = target.parent() {
        let _ = ensure_gitkeep(parent);
    }
    commit_push_library(paths, cfg);
    Ok(())
}

/// Rename a library entry in place, then commit + push the rename
/// (best-effort, Synced only) — the push is part of the change entry, so a
/// caller cannot rename without publishing it.
pub fn rename_entry(
    paths: &crate::config::Paths,
    cfg: &ConfigData,
    rel_path: &str,
    new_name: &str,
) -> AppResult<()> {
    let target = resolve_rel(paths, rel_path)?;
    let name = new_name.trim();
    if !is_plain_entry_name(name) {
        return Err(AppError::Config(format!("invalid name: {name}")));
    }
    let dst = target.parent().unwrap_or_else(|| Path::new(".")).join(name);
    if dst.exists() && dst != target {
        return Err(AppError::Config(format!("{name} already exists")));
    }
    std::fs::rename(&target, &dst)?;
    commit_push_library(paths, cfg);
    Ok(())
}

/// Resolve a `<deviceId>/<sub>/<name>` rel path under the library root and
/// confirm it stays inside (defends against `../`). [`device_subdir`] 的薄壳：
/// 包含性谓词已收口在那里，这里只补「条目必须已存在」这一半（读 / 改 / 删
/// 的既有语义，不存在即 not found）。
fn resolve_rel(paths: &crate::config::Paths, rel_path: &str) -> AppResult<PathBuf> {
    let rel = rel_path.trim().trim_matches('/');
    if rel.is_empty() {
        return Err(AppError::Config("empty library path".into()));
    }
    let (device_id, subpath) = rel.split_once('/').unwrap_or((rel, ""));
    let target = device_subdir(paths, device_id, subpath)?;
    if !target.exists() {
        return Err(AppError::Config(format!(
            "library entry not found: {rel_path}"
        )));
    }
    Ok(target)
}

// ---------------------------------------------------------------------------
// commit + push (best-effort, Synced only)
// ---------------------------------------------------------------------------

/// The git commit message for every library change — one domain constant, so
/// the log reads "cc-one: library sync" no matter which entry pushed.
const COMMIT_MSG: &str = "cc-one: library sync";

/// Stage + commit + push any library change (best-effort, Synced only).
/// Standalone is a no-op — the files already sit in the worktree, nothing to
/// push. Delegates to sync's commit+push core; push failures are logged there,
/// not propagated — the next collect/sync round carries the change up.
/// Called by the change entries themselves ([`upload`] / [`delete_entry`] /
/// [`rename_entry`]) — not by their callers.
pub(crate) fn commit_push_library(paths: &crate::config::Paths, cfg: &ConfigData) {
    crate::sync::commit_and_push_best_effort(paths, cfg, COMMIT_MSG);
}

/// Text-preview cap: files larger than this are not read into the webview
/// (the preview falls back to "too large" instead of loading megabytes).
const TEXT_READ_LIMIT: u64 = 1024 * 1024;

/// Read a library entry as UTF-8 text for the themed text preview.
/// `Some(text)` = readable text; `None` = NOT text (binary, over the size cap,
/// or a directory) — a normal state, not an error. Path safety reuses
/// [`resolve_rel`] (canonicalize + must stay under the library root), so a
/// `../` escape or a missing file is an error, never a read outside the root.
/// Binary probing: a NUL byte in the first 8 KiB means binary.
pub(crate) fn read_text_entry(
    paths: &crate::config::Paths,
    rel_path: &str,
) -> AppResult<Option<String>> {
    let target = resolve_rel(paths, rel_path)?;
    if !target.is_file() {
        return Ok(None);
    }
    if target.metadata().map(|m| m.len()).unwrap_or(u64::MAX) > TEXT_READ_LIMIT {
        return Ok(None);
    }
    let bytes = std::fs::read(&target)?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Ok(None);
    }
    match String::from_utf8(bytes) {
        Ok(text) => Ok(Some(text)),
        Err(_) => Ok(None),
    }
}
