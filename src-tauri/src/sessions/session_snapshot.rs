//! Per-session snapshot — the derived JSONL projection of a favorited session
//! that git syncs. One file per session (`sessions/<id>.jsonl`): a meta line
//! first (system data + favorited + synced_group_id + version `v`), then every
//! message in `(ts, uuid)` order.
//!
//! collect is store-only (it never writes this file). The **push** path
//! materializes a favorited session's file from the store
//! ([`recompute_session_snapshot`]); **pull** reads peers' files back in
//! ([`read_all_session_snapshots`]). Whether a snapshot should exist at all
//! (favorited ⇔ file present) is decided in `snapshot_policy`, not here — this
//! module only owns the byte-stable materialization and tolerant read.

use std::path::Path;

use crate::collect::jsonl::rewrite_jsonl_file;
use crate::config::Paths;
use crate::db::Store;
use crate::error::{AppError, AppResult};
use crate::model::{
    SessionMessage, SessionSnapshotLine, SessionSnapshotMeta, SESSION_SNAPSHOT_VERSION,
};

/// Recompute one session's derived snapshot from the store: the meta line first
/// (system data + favorited + synced_group_id), then every message in
/// `(ts, uuid)` order, as a full file rewrite. The push-side writer — collect no
/// longer appends; the store is the single source of truth and this materializes
/// the snapshot a peer pulls. Byte-stable across pushes ((ts,uuid) order + serde
/// field-declaration order), so the same store yields the same bytes every time
/// (no git churn once a session settles). Returns the message count — the
/// recompute-time snapshot the push path uses to decide whether the session is
/// still clearable after the push lands.
pub fn recompute_session_snapshot(
    store: &Store,
    paths: &Paths,
    device_id: &str,
    session_id: &str,
) -> AppResult<usize> {
    let messages = store.query_session_messages(device_id, session_id)?;
    let count = messages.len();
    let meta = store
        .get_session_snapshot_meta(device_id, session_id)?
        .ok_or_else(|| {
            AppError::Internal(format!("recompute session {session_id}: no meta row"))
        })?;
    let mut lines: Vec<SessionSnapshotLine> = Vec::with_capacity(count + 1);
    lines.push(SessionSnapshotLine::Session(meta));
    for m in messages {
        lines.push(SessionSnapshotLine::Message(m));
    }
    rewrite_jsonl_file(&paths.session_snapshot_path(device_id, session_id), &lines)?;
    Ok(count)
}

/// One parsed session snapshot off disk: the device that authored it, its meta
/// line, and its message lines (file order). The pull path imports these into
/// the store keyed by `(device_id, session_id)`.
#[derive(Debug, Clone)]
pub struct ParsedSessionSnapshot {
    pub device_id: String,
    pub meta: SessionSnapshotMeta,
    pub messages: Vec<SessionMessage>,
}

/// Read every PEER's `sessions/<id>.jsonl` snapshots. `self_device_id`'s own
/// directory is skipped — self is local-authoritative (collect-driven), so its
/// git snapshot must never overwrite fresher local state on pull. Each file's
/// first line is the meta (`type:"session"`); the rest are messages
/// (`type:"message"`). A snapshot whose `v` exceeds
/// [`SESSION_SNAPSHOT_VERSION`] is the upgrade-gate hit: it is skipped with a
/// logged warning (not a hard error — a newer peer's snapshot must not break an
/// older binary's whole pull), so its messages simply do not arrive until the
/// user upgrades. Malformed lines are skipped.
pub fn read_all_session_snapshots(
    paths: &Paths,
    self_device_id: &str,
) -> AppResult<Vec<ParsedSessionSnapshot>> {
    let mut out = Vec::new();
    for dev in crate::devices::iter_data_device_ids(paths)? {
        // Self's snapshots are written by this device's own push — pulling them
        // back would clobber local state with a (possibly stale) git copy.
        if dev == self_device_id {
            continue;
        }
        let sess_dir = paths.device_data_dir(&dev).join("sessions");
        if !sess_dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&sess_dir)? {
            let f = f?;
            if f.file_type()?.is_file() {
                if let Some(snap) = read_one_session_snapshot(&f.path(), &dev) {
                    out.push(snap);
                }
            }
        }
    }
    Ok(out)
}

/// Parse one `sessions/<id>.jsonl`: the meta line first, then messages. Returns
/// `None` for a file whose meta line carries a higher `v` than this binary
/// supports (upgrade gate), or one with no meta line at all (not a snapshot).
/// Read errors and malformed lines are tolerated — a corrupt peer file must not
/// abort a pull.
fn read_one_session_snapshot(path: &Path, device_id: &str) -> Option<ParsedSessionSnapshot> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return None,
    };
    let mut meta: Option<SessionSnapshotMeta> = None;
    let mut messages = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<SessionSnapshotLine>(line) {
            Ok(SessionSnapshotLine::Session(m)) => {
                if m.v > SESSION_SNAPSHOT_VERSION {
                    eprintln!(
                        "[cc-one] session snapshot {} is v{} (this build supports v{}); skipping — upgrade to read it",
                        m.id, m.v, SESSION_SNAPSHOT_VERSION
                    );
                    return None;
                }
                meta = Some(m);
            }
            Ok(SessionSnapshotLine::Message(m)) => messages.push(m),
            Err(_) => continue,
        }
    }
    meta.map(|m| ParsedSessionSnapshot {
        device_id: device_id.to_string(),
        meta: m,
        messages,
    })
}
