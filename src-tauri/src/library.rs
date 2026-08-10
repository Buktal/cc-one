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

use std::path::{Path, PathBuf};

use crate::config::{ConfigData, ConfigStore};
use crate::db::Store;
use crate::error::{AppError, AppResult};

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
    let lib_root = paths.library.clone();

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
        let dir = lib_root.join(&did).join(subpath_rel(subpath));
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

/// Subpath normalised to a relative PathBuf (empty ⇒ device root).
fn subpath_rel(subpath: &str) -> PathBuf {
    let trimmed = subpath.trim().trim_matches('/');
    if trimmed.is_empty() {
        PathBuf::new()
    } else {
        PathBuf::from(trimmed)
    }
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
/// The caller commits + pushes after a successful batch.
pub fn upload(
    paths: &crate::config::Paths,
    cfg: &ConfigData,
    items: &[UploadItem],
    subpath: &str,
) -> AppResult<()> {
    if cfg.device_id.is_empty() {
        return Err(AppError::Config("device id not initialized".into()));
    }
    let dest_dir = paths
        .library
        .join(&cfg.device_id)
        .join(subpath_rel(subpath));
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
        if name.is_empty() || name.contains('/') || name.contains('\\') || name == ".gitkeep" {
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

/// Delete a library entry (file or dir). The caller commits + pushes.
pub fn delete_entry(paths: &crate::config::Paths, rel_path: &str) -> AppResult<()> {
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
    Ok(())
}

/// Rename a library entry in place. The caller commits + pushes.
pub fn rename_entry(paths: &crate::config::Paths, rel_path: &str, new_name: &str) -> AppResult<()> {
    let target = resolve_rel(paths, rel_path)?;
    let name = new_name.trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return Err(AppError::Config(format!("invalid name: {name}")));
    }
    let dst = target.parent().unwrap_or_else(|| Path::new(".")).join(name);
    if dst.exists() && dst != target {
        return Err(AppError::Config(format!("{name} already exists")));
    }
    std::fs::rename(&target, &dst)?;
    Ok(())
}

/// Resolve a `<deviceId>/<sub>/<name>` rel path under the library root, then
/// canonicalize and confirm it stays inside the root (defends against `../`).
fn resolve_rel(paths: &crate::config::Paths, rel_path: &str) -> AppResult<PathBuf> {
    let rel = rel_path.trim().trim_matches('/');
    if rel.is_empty() {
        return Err(AppError::Config("empty library path".into()));
    }
    let p = paths.library.join(rel);
    let canon = p
        .canonicalize()
        .map_err(|_| AppError::Config(format!("library entry not found: {rel_path}")))?;
    let root_canon = paths
        .library
        .canonicalize()
        .unwrap_or_else(|_| paths.library.clone());
    if !canon.starts_with(&root_canon) {
        return Err(AppError::Config("library path escapes the root".into()));
    }
    Ok(canon)
}

// ---------------------------------------------------------------------------
// device forget (migrate / delete a peer's subtree)
// ---------------------------------------------------------------------------

/// Sanitise a peer display name into a safe `from-<name>` folder label,
/// falling back to the device id when the name is empty. Path separators and
/// Windows-reserved chars become `_`; length is capped.
fn migrate_folder_name(name: &str, fallback_id: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string();
    let base = if cleaned.is_empty() {
        fallback_id.to_string()
    } else {
        cleaned.chars().take(64).collect()
    };
    format!("from-{base}")
}

/// Apply a [`LibraryForgetAction`] to a peer's library subtree. Local-only —
/// no Git push (matches `forget_device`'s existing `repo/data/<id>/` cleanup;
/// a peer still active elsewhere reappears on the next sync). `Migrate` moves
/// the subtree into THIS device's library under `from-<peerName>/` (a unique
/// suffix is appended on collision so repeated migrations never overwrite).
/// No-op when the peer has no library subtree.
pub fn forget_device_library(
    paths: &crate::config::Paths,
    cfg: &ConfigData,
    peer_id: &str,
    action: LibraryForgetAction,
    peer_name: &str,
) -> AppResult<()> {
    let peer_dir = paths.library.join(peer_id);
    if !peer_dir.exists() {
        return Ok(());
    }
    match action {
        LibraryForgetAction::Delete => {
            std::fs::remove_dir_all(&peer_dir)?;
        }
        LibraryForgetAction::Migrate => {
            let self_root = paths.library.join(&cfg.device_id);
            std::fs::create_dir_all(&self_root)?;
            let folder = migrate_folder_name(peer_name, peer_id);
            let mut target = self_root.join(&folder);
            let mut n = 2;
            while target.exists() {
                target = self_root.join(format!("{folder}-{n}"));
                n += 1;
            }
            std::fs::rename(&peer_dir, &target)?;
        }
    }
    Ok(())
}

/// Recursively count files (excl. `.gitkeep`) and folders under a device's
/// library subtree; `{0, 0}` when it does not exist.
pub(crate) fn count_subtree(dir: &Path) -> DeviceLibrarySummary {
    fn walk(dir: &Path, files: &mut f64, dirs: &mut f64) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy() == ".gitkeep" {
                continue;
            }
            let Ok(ft) = e.file_type() else {
                continue;
            };
            if ft.is_dir() {
                *dirs += 1.0;
                walk(&e.path(), files, dirs);
            } else {
                *files += 1.0;
            }
        }
    }
    let mut files = 0.0;
    let mut dirs = 0.0;
    if dir.is_dir() {
        walk(dir, &mut files, &mut dirs);
    }
    DeviceLibrarySummary { files, dirs }
}

// ---------------------------------------------------------------------------
// commit + push (best-effort, Synced only)
// ---------------------------------------------------------------------------

/// Stage + commit + push any library change (best-effort, Synced only).
/// Standalone is a no-op — the files already sit in the worktree, nothing to
/// push. Delegates to sync's commit+push core; push failures are logged there,
/// not propagated — the next collect/sync round carries the change up.
pub(crate) fn commit_push_library(paths: &crate::config::Paths, cfg: &ConfigData) {
    crate::sync::commit_and_push_best_effort(paths, cfg, "cc-one: library sync");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    fn self_cfg() -> ConfigData {
        ConfigData {
            device_id: "aabbccddeeff".to_string(),
            ..Default::default()
        }
    }

    fn write_file(p: &Path, body: &str) {
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn read_text_entry_returns_utf8_text() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let dev = "aabbccddeeff";
        let dir = paths.library.join(dev);
        std::fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("notes.md"), "# Hello\n\n正文内容");
        let text = read_text_entry(&paths, &format!("{dev}/notes.md")).unwrap();
        assert_eq!(text.as_deref(), Some("# Hello\n\n正文内容"));
    }

    #[test]
    fn read_text_entry_returns_none_for_binary_and_oversized() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let dev = "aabbccddeeff";
        let dir = paths.library.join(dev);
        std::fs::create_dir_all(&dir).unwrap();
        // NUL byte in the head ⇒ binary.
        write_file(&dir.join("bin.dat"), "ok\0binary");
        assert_eq!(
            read_text_entry(&paths, &format!("{dev}/bin.dat")).unwrap(),
            None,
            "NUL in the first 8 KiB ⇒ not text"
        );
        // Over the 1 MiB cap ⇒ None (not an error).
        let big = "x".repeat((TEXT_READ_LIMIT + 1) as usize);
        write_file(&dir.join("big.txt"), &big);
        assert_eq!(
            read_text_entry(&paths, &format!("{dev}/big.txt")).unwrap(),
            None,
            "oversized file ⇒ not loaded"
        );
        // A directory is not text.
        assert_eq!(
            read_text_entry(&paths, dev).unwrap(),
            None,
            "directory ⇒ None"
        );
    }

    #[test]
    fn read_text_entry_rejects_escape_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        // `..` escapes the library root.
        std::fs::create_dir_all(paths.library.join("aabbccddeeff")).unwrap();
        let err = read_text_entry(&paths, "../secret").unwrap_err();
        assert!(
            err.to_string().contains("escapes") || err.to_string().contains("not found"),
            "path escape rejected: {err}"
        );
        // Missing file errors (resolve_rel canonicalize fails).
        assert!(read_text_entry(&paths, "aabbccddeeff/nope.txt").is_err());
    }

    #[test]
    fn migrate_moves_subtree_into_self() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = self_cfg();
        let peer = "112233445566";

        let peer_dir = paths.library.join(peer);
        std::fs::create_dir_all(peer_dir.join("sub")).unwrap();
        write_file(&peer_dir.join("note.txt"), "hi");
        write_file(&peer_dir.join("sub").join("inner.txt"), "yo");
        write_file(&peer_dir.join(".gitkeep"), "");

        forget_device_library(&paths, &cfg, peer, LibraryForgetAction::Migrate, "MacBook").unwrap();

        // Peer subtree gone; contents now under self/from-MacBook/.
        assert!(!peer_dir.exists());
        let moved = paths.library.join(&cfg.device_id).join("from-MacBook");
        assert!(moved.is_dir());
        assert_eq!(
            std::fs::read_to_string(moved.join("note.txt")).unwrap(),
            "hi"
        );
        assert!(moved.join("sub").is_dir());
        assert_eq!(
            std::fs::read_to_string(moved.join("sub").join("inner.txt")).unwrap(),
            "yo"
        );
    }

    #[test]
    fn migrate_collision_appends_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = self_cfg();
        let peer = "112233445566";

        // Pre-existing migrate target must NOT be overwritten.
        let existing = paths.library.join(&cfg.device_id).join("from-MacBook");
        std::fs::create_dir_all(&existing).unwrap();
        write_file(&existing.join("keeper.txt"), "keep");

        let peer_dir = paths.library.join(peer);
        std::fs::create_dir_all(&peer_dir).unwrap();
        write_file(&peer_dir.join("note.txt"), "hi");

        forget_device_library(&paths, &cfg, peer, LibraryForgetAction::Migrate, "MacBook").unwrap();

        assert_eq!(
            std::fs::read_to_string(existing.join("keeper.txt")).unwrap(),
            "keep"
        );
        let moved = paths.library.join(&cfg.device_id).join("from-MacBook-2");
        assert_eq!(
            std::fs::read_to_string(moved.join("note.txt")).unwrap(),
            "hi"
        );
    }

    #[test]
    fn delete_removes_subtree() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = self_cfg();
        let peer = "112233445566";

        let peer_dir = paths.library.join(peer);
        std::fs::create_dir_all(peer_dir.join("sub")).unwrap();
        write_file(&peer_dir.join("note.txt"), "hi");

        forget_device_library(&paths, &cfg, peer, LibraryForgetAction::Delete, "MacBook").unwrap();

        assert!(!peer_dir.exists());
    }

    #[test]
    fn forget_is_noop_when_peer_has_no_library() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let cfg = self_cfg();
        // No library/<peer>/ exists for either action.
        forget_device_library(
            &paths,
            &cfg,
            "112233445566",
            LibraryForgetAction::Delete,
            "MacBook",
        )
        .unwrap();
        forget_device_library(
            &paths,
            &cfg,
            "112233445566",
            LibraryForgetAction::Migrate,
            "MacBook",
        )
        .unwrap();
        // Migrate must not spuriously create the self root on a no-op.
        assert!(!paths.library.join(&cfg.device_id).exists());
    }

    #[test]
    fn count_subtree_excludes_gitkeep() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let peer_dir = paths.library.join("112233445566");
        std::fs::create_dir_all(peer_dir.join("d1")).unwrap();
        write_file(&peer_dir.join("a.txt"), "a");
        write_file(&peer_dir.join(".gitkeep"), "");
        write_file(&peer_dir.join("d1").join("b.txt"), "b");

        let s = count_subtree(&peer_dir);
        assert_eq!(s.files, 2.0); // a.txt + d1/b.txt (.gitkeep excluded)
        assert_eq!(s.dirs, 1.0); // d1
    }

    #[test]
    fn count_subtree_missing_dir_is_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::resolve(tmp.path());
        let s = count_subtree(&paths.library.join("nope"));
        assert_eq!(s.files, 0.0);
        assert_eq!(s.dirs, 0.0);
    }

    #[test]
    fn migrate_folder_name_sanitises() {
        assert_eq!(
            migrate_folder_name("MacBook", "112233445566"),
            "from-MacBook"
        );
        assert_eq!(migrate_folder_name("a/b\\c", "112233445566"), "from-a_b_c");
        assert_eq!(migrate_folder_name("", "112233445566"), "from-112233445566");
        assert_eq!(
            migrate_folder_name("   ", "112233445566"),
            "from-112233445566"
        );
    }
}
