//! The pull-side coarse gate: which per-device directories a pull re-reads.
//!
//! Pulls run on the scheduler whether or not any peer pushed, so most pulls
//! see a worktree identical to the previous one — re-reading it (and for
//! sessions, re-parsing every snapshot line) is pure waste. The gate keys a
//! domain's read on a per-directory SIGNATURE: `file name → (mtime_nanos,
//! len)` for exactly the files that read consumes, computed from `read_dir` +
//! metadata only, so an unchanged dir costs a directory listing, never a file
//! read. Both pull imports share the mechanism here (`domains::usage_import`
//! over `data/<id>/`, `domains::sessions_import` over
//! `data/<id>/sessions/`); each domain supplies its own subtree path and file
//! predicate, so this module knows no domain.
//!
//! The gate only saves reads; it never changes semantics:
//! - Whatever IS re-read is backstopped by the store's primary keys
//!   (`(uuid, device_id)` for usage rows, `(device_id, uuid)` for messages).
//! - A landed git checkout invalidates the whole cache
//!   (`flow::pull_and_import` → `Store::invalidate_pull_dir_sigs`): the
//!   checkout just rewrote the worktree, so restored files are re-read even
//!   when git wrote back byte-identical content — this is what keeps the
//!   forget-device-then-repull round-trip working.
//! - The signature is deliberately coarse (mtime + length, no content hash):
//!   a file rewritten inside the filesystem's mtime-granularity tick with an
//!   unchanged length could be missed until its next rewrite (observed on
//!   Windows: rapid rewrites can keep the same mtime; a real content change
//!   nearly always changes the length, which the signature catches — tests
//!   therefore drive changes through LENGTH, never a presumed mtime refresh).
//!   With the checkout invalidation above, git — the only writer that
//!   mutates peer files — no longer relies on mtime at all; the residual
//!   window covers only non-git rewrites of already-read files, and whatever
//!   is re-read is in any case absorbed by the primary keys.
//!
//! The cache is in-memory on the Store (`Store::pull_dir_sigs`): a restart
//! re-reads once and the primary keys absorb it. Deliberately not persisted
//! to SQLite — the cache is an optimization; losing it is free.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use crate::error::AppResult;

/// One directory's signature — the gate's compared state: `file name →
/// (mtime_nanos, len)` for every regular file the reading domain's predicate
/// accepts; empty for an absent dir. Computed without reading file content,
/// so a signature check costs a directory listing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DirSig {
    files: BTreeMap<String, (u128, u64)>,
}

impl DirSig {
    /// The tracked file names, sorted. A domain's cheap presence half (e.g.
    /// which session ids still have a snapshot file) is derived from these —
    /// the same enumeration the gate compares, so a gated (skipped) dir still
    /// reconciles.
    pub(crate) fn file_names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

/// Compute `dir`'s [`DirSig`] over the regular files `keep` accepts. `keep`
/// is the reading domain's file-shape test and must accept exactly the files
/// its read consumes (usage: `<usage|turns>-*.jsonl`; sessions: every regular
/// file — the tolerant reader consumes them all). An absent (or non-dir) path
/// yields the empty signature — caching it makes a LATER-appearing dir read
/// on the next pull, because its non-empty signature differs.
pub(crate) fn dir_sig(dir: &Path, keep: fn(&Path) -> bool) -> AppResult<DirSig> {
    if !dir.is_dir() {
        return Ok(DirSig::default());
    }
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if !keep(&path) {
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
    Ok(DirSig { files })
}

/// One pull's gate over the Store's cached dir signatures (`Store::
/// pull_dir_sigs`). The protocol, per directory: compute the fresh [`DirSig`]
/// and ask [`DirGate::unchanged`] — on a hit the read is skipped entirely —
/// on a miss perform the domain's read and collect the `(dir, sig)` pair;
/// once the store writes that read fed have ALL succeeded, vouch for the
/// whole batch with [`DirGate::observe_all`]. Vouching only after success
/// means a failed read or import leaves the cache stale, so the next pull
/// retries the read instead of skipping it forever.
pub(crate) struct DirGate<'a> {
    sigs: MutexGuard<'a, HashMap<PathBuf, DirSig>>,
}

impl<'a> DirGate<'a> {
    /// Lock the Store's signature cache for one pull's gate.
    pub(crate) fn new(sigs: MutexGuard<'a, HashMap<PathBuf, DirSig>>) -> Self {
        Self { sigs }
    }

    /// Does `dir` look exactly as it did at the last vouched read? A cache
    /// miss counts as changed — the first pull reads everything, and the
    /// in-memory cache means a restart re-reads once (the primary keys
    /// absorb it).
    pub(crate) fn unchanged(&self, dir: &Path, sig: &DirSig) -> bool {
        self.sigs.get(dir) == Some(sig)
    }

    /// Vouch for every `(dir, sig)` pair: record it as the dir's
    /// as-of-last-read state, so the next pull skips unchanged dirs. Call
    /// once per pull, after the store writes fed by the gated reads have all
    /// succeeded (see the protocol above).
    pub(crate) fn observe_all(&mut self, read: Vec<(PathBuf, DirSig)>) {
        for (dir, sig) in read {
            self.sigs.insert(dir, sig);
        }
    }
}

#[cfg(test)]
mod tests;
