//! 对端遗忘（forget_device）在 library 子树上的落点：整体删除，或迁入本机
//! library 收留。Local-only，不触 Git push（与 `forget_device` 对
//! `repo/data/<id>/` 的既有清理同语义——对端若仍活跃于仓库，下一次 sync 会
//! 原样回来）。

use std::path::Path;

use crate::config::{ConfigData, Paths};
use crate::error::{AppError, AppResult};

use super::LibraryForgetAction;

/// Sanitise a peer display name into a safe `from-<name>` folder label,
/// falling back to the device id when the name is empty. Path separators and
/// Windows-reserved chars become `_`; length is capped.
pub(super) fn migrate_folder_name(name: &str, fallback_id: &str) -> String {
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
    paths: &Paths,
    cfg: &ConfigData,
    peer_id: &str,
    action: LibraryForgetAction,
    peer_name: &str,
) -> AppResult<()> {
    // peer_id 与其它入口的路径参数同源（前端命令参数），必须过同一词法
    // 谓词——`..` 会让下面的 remove_dir_all / rename 作用到 library root 之外。
    if !super::has_only_plain_components(Path::new(peer_id)) {
        return Err(AppError::Config(format!(
            "library path escapes the root: {peer_id}"
        )));
    }
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
pub(crate) fn count_subtree(dir: &Path) -> super::DeviceLibrarySummary {
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
    super::DeviceLibrarySummary { files, dirs }
}
