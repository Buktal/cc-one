//! OpenCode (`~/.local/share/opencode/opencode.db`) session-log parser.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::model::{RawSession, SessionMessage, SessionMessageRole, TokenCounts};
use crate::time::epoch_millis_to_iso;

use super::{
    metadata_modified_nanos, truncate, CollectResult, FileCursor, GateMode, RawUsage, ScanProgress,
    ScanProgressDelta, SourceParser, TRIM_LIMIT,
};

/// Stable source tag — becomes `RawUsage.source` / `RawSession.source` and the
/// DB source column; the single literal behind `name()`, usage, and session
/// construction.
const SOURCE_TAG: &str = "opencode";

/// OpenCode (`~/.local/share/opencode/opencode.db`) session-log parser.
///
/// OpenCode stores sessions in a SQLite db (WAL mode). `message.data` is a JSON
/// string with Anthropic-style tokens: `input` is fresh, `cache.{read,write}`
/// are separate, `reasoning` folds into `output`. The parser opens the db
/// read-only and queries per session. The main db file only updates on
/// checkpoint, so fresh commits in `-wal` are merged into the mtime gate; a
/// two-level watermark (file + per-session `time_updated`) skips unchanged work.
pub struct OpenCodeSourceParser {
    db_path: Option<PathBuf>,
}

impl OpenCodeSourceParser {
    /// Root-injection seam: parser whose db path falls back to
    /// `home/.local/share/opencode/opencode.db` (env overrides still win). The
    /// collect orchestration factory (`all_source_parsers_at`) builds every
    /// parser through this seam, so tests can point the whole chain at a
    /// tempdir fixture instead of the real `~`.
    pub(crate) fn new_at(home: &Path) -> Self {
        Self {
            db_path: opencode_db_path_at(home),
        }
    }

    /// Test/override constructor with an explicit db path.
    #[cfg(test)]
    pub(crate) fn with_db(path: PathBuf) -> Self {
        Self {
            db_path: Some(path),
        }
    }
}

impl SourceParser for OpenCodeSourceParser {
    fn name(&self) -> &'static str {
        SOURCE_TAG
    }

    fn discover(&self) -> AppResult<Vec<PathBuf>> {
        match &self.db_path {
            Some(p) if p.exists() => Ok(vec![p.clone()]),
            _ => Ok(Vec::new()),
        }
    }

    fn parse(&self, files: &[PathBuf]) -> AppResult<CollectResult> {
        let mut events = Vec::new();
        let mut sessions = Vec::new();
        let mut messages = Vec::new();
        let mut skipped = 0u32;
        let mut files_scanned = 0u32;
        for db_path in files {
            files_scanned += 1;
            let conn = match open_opencode_readonly(db_path) {
                Ok(c) => c,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            match collect_all(&conn) {
                Ok((ev, ss, ms)) => {
                    events.extend(ev);
                    sessions.extend(ss);
                    messages.extend(ms);
                }
                Err(_) => skipped += 1,
            }
        }
        super::order_results(&mut events, &mut sessions);
        Ok(CollectResult {
            source: self.name().to_string(),
            events,
            turn_durations: Vec::new(),
            sessions,
            messages,
            files_scanned,
            lines_skipped: skipped,
            // SQLite session table is self-managed (per-session watermark) —
            // file-backed reconciliation must not touch it.
            session_ids: Vec::new(),
        })
    }

    /// Two-level watermark incremental: file-level mtime gate (db + `-wal`
    /// merged) skips an unchanged db; per-session `time_updated` skips sessions
    /// already synced. A session with an in-progress message (no `time.completed`)
    /// does not advance its cursor, so it retries next collect.
    fn collect_incremental(
        &self,
        progress: &ScanProgress,
    ) -> AppResult<(CollectResult, ScanProgressDelta)> {
        let mut result = CollectResult {
            source: self.name().to_string(),
            // SQLite session table is self-managed — no file-backed reconcile.
            session_ids: Vec::new(),
            ..CollectResult::default()
        };
        let mut delta = ScanProgressDelta::new();
        let Some(db_path) = &self.db_path else {
            return Ok((result, delta));
        };
        let db_path_str = super::scan_progress_key(db_path);

        let Some(merged_mtime) = merged_db_mtime(db_path) else {
            return Ok((result, delta));
        };
        result.files_scanned = 1;
        let prev_file = progress.get(&db_path_str).copied().unwrap_or_default();
        if prev_file.last_modified != 0 && merged_mtime <= prev_file.last_modified {
            return Ok((result, delta));
        }

        let conn = match open_opencode_readonly(db_path) {
            Ok(c) => c,
            Err(_) => {
                result.lines_skipped = 1;
                return Ok((result, delta));
            }
        };
        let sessions = match query_sessions(&conn) {
            Ok(s) => s,
            Err(_) => {
                result.lines_skipped = 1;
                return Ok((result, delta));
            }
        };
        let session_rows = match query_session_rows(&conn) {
            Ok(r) => r,
            Err(_) => {
                result.lines_skipped = 1;
                return Ok((result, delta));
            }
        };
        let row_by_id: std::collections::HashMap<&str, &OpenCodeSessionRow> =
            session_rows.iter().map(|r| (r.id.as_str(), r)).collect();
        for (session_id, watermark) in &sessions {
            let sync_key = format!("{db_path_str}:{session_id}");
            let prev_sess = progress.get(&sync_key).copied().unwrap_or_default();
            if *watermark <= prev_sess.last_modified {
                continue;
            }
            // System data is refreshable — re-emit the RawSession whenever the
            // session advances (mirrors Claude rebuilding session meta from a
            // file that passed its mtime gate).
            if let Some(row) = row_by_id.get(session_id.as_str()) {
                result.sessions.push(opencode_raw_session(row));
            }
            match query_assistant_messages(&conn, session_id) {
                Ok(qr) => {
                    for (message_id, msg) in &qr.messages {
                        result
                            .events
                            .push(opencode_raw_usage(session_id, message_id, msg));
                    }
                    if !qr.has_incomplete_usage {
                        delta.insert(
                            sync_key.clone(),
                            FileCursor {
                                last_modified: *watermark,
                                last_line_offset: 0,
                            },
                        );
                    }
                }
                Err(_) => result.lines_skipped += 1,
            }
            // Transcript (all roles) — re-queried on each advance and deduped
            // downstream by the stable message-id uuid, so re-emission is a
            // no-op for already-synced lines. Same shape as the usage re-emit
            // above.
            match query_transcript_messages(&conn, session_id) {
                Ok(ms) => result.messages.extend(ms),
                Err(_) => result.lines_skipped += 1,
            }
        }
        super::order_results(&mut result.events, &mut result.sessions);
        delta.insert(
            db_path_str,
            FileCursor {
                last_modified: merged_mtime,
                last_line_offset: 0,
            },
        );
        Ok((result, delta))
    }

    /// SQLite-backed: the incremental gate is a per-session watermark (db
    /// mtime + per-session `time_updated`), not a line cursor — declared so
    /// the shared driver's line-cursor contract is not assumed for this
    /// parser. Its own `collect_incremental` does not consult this; the
    /// declaration pins the strategy where a future driver change can see it.
    fn gate_mode(&self, _file: &Path) -> GateMode {
        GateMode::SessionWatermark
    }
}

/// Resolve the opencode db path: `OPENCODE_DB` (absolute) > `XDG_DATA_HOME` >
/// `~/.local/share/opencode/opencode.db`. OpenCode uses xdg-basedir uniformly
/// across platforms, so this is the same path on Windows as on Linux. The
/// home-dir fallback is injected (`home`) — the root-injection seam
/// (`OpenCodeSourceParser::new_at`) used by the collect orchestration factory;
/// production resolves the real home.
fn opencode_db_path_at(home: &Path) -> Option<PathBuf> {
    if let Ok(v) = std::env::var("OPENCODE_DB") {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            return Some(p);
        }
    }
    if let Ok(v) = std::env::var("XDG_DATA_HOME") {
        let p = PathBuf::from(v);
        if p.is_absolute() {
            return Some(p.join("opencode").join("opencode.db"));
        }
    }
    Some(
        home.join(".local")
            .join("share")
            .join("opencode")
            .join("opencode.db"),
    )
}

/// max(db mtime, db-wal mtime) — the main db only updates on checkpoint, so
/// fresh commits in the `-wal` side file must be considered or they're missed.
fn merged_db_mtime(db_path: &Path) -> Option<i64> {
    let db_meta = std::fs::metadata(db_path).ok()?;
    let mut m = metadata_modified_nanos(&db_meta);
    let wal = db_path.with_extension("db-wal");
    if let Ok(wal_meta) = std::fs::metadata(&wal) {
        m = m.max(metadata_modified_nanos(&wal_meta));
    }
    Some(m)
}

fn open_opencode_readonly(db_path: &Path) -> AppResult<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| AppError::Db(format!("cannot open opencode.db read-only: {e}")))
}

/// Full-scan collect: every session's [`RawSession`] + completed-assistant
/// usage events + transcript messages (no watermark gate). Reached only from
/// [`SourceParser::parse`] — the test/diagnostic full-scan path. Production runs
/// [`OpenCodeSourceParser::collect_incremental`] (per-session, two-level watermark).
#[allow(dead_code)] // parse-only; production runs collect_incremental
fn collect_all(
    conn: &rusqlite::Connection,
) -> AppResult<(Vec<RawUsage>, Vec<RawSession>, Vec<SessionMessage>)> {
    let session_rows = query_session_rows(conn)?;
    let mut events = Vec::new();
    let mut sessions = Vec::new();
    let mut messages = Vec::new();
    for row in &session_rows {
        sessions.push(opencode_raw_session(row));
        if let Ok(qr) = query_assistant_messages(conn, &row.id) {
            for (message_id, msg) in &qr.messages {
                events.push(opencode_raw_usage(&row.id, message_id, msg));
            }
        }
        if let Ok(ms) = query_transcript_messages(conn, &row.id) {
            messages.extend(ms);
        }
    }
    Ok((events, sessions, messages))
}

/// One row of the opencode `session` table — the metadata needed to build a
/// [`RawSession`] (system data), separate from the sync watermark used by the
/// incremental gate ([`query_sessions`]).
struct OpenCodeSessionRow {
    id: String,
    title: String,
    directory: String,
    time_created_ms: i64,
    time_updated_ms: i64,
}

/// All session rows (metadata only; the sync watermark comes from
/// [`query_sessions`]). Ordered by `(time_updated, id)` for deterministic
/// source ordering; callers re-sort the emitted [`RawSession`]s by
/// `(last_active_at, id)` before returning.
fn query_session_rows(conn: &rusqlite::Connection) -> AppResult<Vec<OpenCodeSessionRow>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, title, directory, time_created, time_updated
             FROM session
             ORDER BY time_updated, id",
        )
        .map_err(|e| AppError::Db(format!("opencode session-row query prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(OpenCodeSessionRow {
                id: row.get(0)?,
                title: row.get(1)?,
                directory: row.get(2)?,
                time_created_ms: row.get(3)?,
                time_updated_ms: row.get(4)?,
            })
        })
        .map_err(|e| AppError::Db(format!("opencode session-row query: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| AppError::Db(format!("opencode session-row row: {e}")))?);
    }
    Ok(out)
}

/// Build a [`RawSession`] (system data) from a session row. `title_orig` falls
/// back from the DB title to the directory's basename, then to empty — matching
/// CC-Switch's OpenCode title derivation. Timestamps are ms-epoch → ISO8601 UTC
/// via the shared [`epoch_millis_to_iso`].
fn opencode_raw_session(row: &OpenCodeSessionRow) -> RawSession {
    RawSession {
        id: row.id.clone(),
        source: SOURCE_TAG.to_string(),
        project_dir: row.directory.clone(),
        title_orig: derive_title(&row.title, &row.directory),
        started_at: epoch_millis_to_iso(row.time_created_ms),
        last_active_at: epoch_millis_to_iso(row.time_updated_ms),
        agent_type: String::new(),
    }
}

/// Title fallback chain: non-empty DB title > directory basename > empty
/// string. (OpenCode titles are short by nature — the DB column or a basename
/// — so unlike Claude's summary/first-prompt title no truncation cap is needed.)
fn derive_title(title: &str, directory: &str) -> String {
    let t = title.trim();
    if !t.is_empty() {
        return t.to_string();
    }
    Path::new(directory)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

/// A session's transcript (all roles), reconstructed by joining `message` with
/// its `part` rows. One [`SessionMessage`] per message row: `uuid` = the
/// message id (stable → re-emission is idempotent under ingest's uuid dedup),
/// `role` mapped from `data.role`, `content` from parts joined in
/// `time_created` order (text → text, tool → tool name), `name` = the first
/// tool part's tool name when present. Empty-content and unknown-role messages
/// are skipped.
fn query_transcript_messages(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> AppResult<Vec<SessionMessage>> {
    // 1. Collect this session's parts, grouped by message_id (one pass; the
    //    block scope releases the statement borrow before the message query).
    let mut parts_by_msg: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created",
            )
            .map_err(|e| AppError::Db(format!("opencode part query prepare: {e}")))?;
        let rows = stmt
            .query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| AppError::Db(format!("opencode part query: {e}")))?;
        for row in rows {
            let (message_id, data) =
                row.map_err(|e| AppError::Db(format!("opencode part row: {e}")))?;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&data) {
                parts_by_msg.entry(message_id).or_default().push(v);
            }
        }
    }

    // 2. Walk messages in time order, joining their parts.
    let mut out = Vec::new();
    {
        let mut stmt = conn
            .prepare(
                "SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY time_created",
            )
            .map_err(|e| AppError::Db(format!("opencode transcript query prepare: {e}")))?;
        let rows = stmt
            .query_map([session_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| AppError::Db(format!("opencode transcript query: {e}")))?;
        for row in rows {
            let (id, time_created_ms, data_json) =
                row.map_err(|e| AppError::Db(format!("opencode transcript row: {e}")))?;
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&data_json) else {
                continue;
            };
            let role_str = value.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let Some(role) = map_role(role_str) else {
                continue;
            };
            let model = value
                .get("modelID")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let empty = Vec::new();
            let part_values = parts_by_msg.get(&id).unwrap_or(&empty);
            let (content, name) = build_content_and_name(part_values);
            if content.trim().is_empty() {
                continue;
            }
            out.push(SessionMessage {
                uuid: id,
                session_id: session_id.to_string(),
                role,
                ts: epoch_millis_to_iso(time_created_ms),
                model,
                name,
                content: truncate(&content, TRIM_LIMIT),
            });
        }
    }
    Ok(out)
}

/// Map an OpenCode `data.role` string to a [`SessionMessageRole`]. Unknown
/// values map to `None` so the caller skips the message rather than emitting a
/// mis-classified line.
fn map_role(role: &str) -> Option<SessionMessageRole> {
    match role {
        "user" => Some(SessionMessageRole::User),
        "assistant" => Some(SessionMessageRole::Assistant),
        "tool" => Some(SessionMessageRole::Tool),
        _ => None,
    }
}

/// Build `(content, name)` from a message's part JSON values (in order):
/// - `text` part → its text appended to `content`;
/// - `tool` part → the tool name appended to `content` AND captured as `name`;
/// - any other part type → skipped.
fn build_content_and_name(parts: &[serde_json::Value]) -> (String, Option<String>) {
    let mut pieces: Vec<String> = Vec::new();
    let mut tool_name: Option<String> = None;
    for v in parts {
        match v.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(t) = v.get("text").and_then(|x| x.as_str()) {
                    if !t.trim().is_empty() {
                        pieces.push(t.to_string());
                    }
                }
            }
            Some("tool") => {
                if let Some(name) = v.get("tool").and_then(|x| x.as_str()) {
                    if !name.is_empty() {
                        pieces.push(name.to_string());
                        if tool_name.is_none() {
                            tool_name = Some(name.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (pieces.join("\n"), tool_name)
}

/// Per-session (id, sync watermark) — the max of the session's own
/// `time_updated` and all its messages' `time_updated`.
fn query_sessions(conn: &rusqlite::Connection) -> AppResult<Vec<(String, i64)>> {
    let mut stmt = conn
        .prepare(
            "SELECT s.id,
                    MAX(s.time_updated, COALESCE(MAX(m.time_updated), s.time_updated)) AS sync_watermark
             FROM session s
             LEFT JOIN message m ON m.session_id = s.id
             GROUP BY s.id
             ORDER BY sync_watermark",
        )
        .map_err(|e| AppError::Db(format!("opencode session query prepare: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| AppError::Db(format!("opencode session query: {e}")))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(|e| AppError::Db(format!("opencode session row: {e}")))?);
    }
    Ok(out)
}

/// A session's completed assistant messages, plus whether an in-progress
/// message (no `time.completed`) was seen — the caller retries that session.
struct OpenCodeMessageQuery {
    messages: Vec<(String, OpenCodeMessageData)>,
    has_incomplete_usage: bool,
}

/// Parsed `message.data` token fields (Anthropic-style: fresh input, cache split).
struct OpenCodeMessageData {
    input_tokens: u32,
    output_tokens: u32,
    reasoning_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    model_id: String,
    timestamp_ms: i64,
}

fn query_assistant_messages(
    conn: &rusqlite::Connection,
    session_id: &str,
) -> AppResult<OpenCodeMessageQuery> {
    let mut stmt = conn
        .prepare("SELECT id, data FROM message WHERE session_id = ?1 ORDER BY time_created")
        .map_err(|e| AppError::Db(format!("opencode message query prepare: {e}")))?;
    let rows = stmt
        .query_map([session_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| AppError::Db(format!("opencode message query: {e}")))?;
    let mut messages = Vec::new();
    let mut has_incomplete_usage = false;
    for row in rows {
        let (message_id, data_json) =
            row.map_err(|e| AppError::Db(format!("opencode message row: {e}")))?;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&data_json) else {
            continue;
        };
        if value.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        if value.get("tokens").is_none() {
            continue;
        }
        // In-progress messages carry half-formed tokens and no `time.completed`;
        // skip them and signal the caller to retry the session.
        if value.get("time").and_then(|t| t.get("completed")).is_none() {
            has_incomplete_usage = true;
            continue;
        }
        if let Some(msg) = parse_opencode_message_data(&value) {
            messages.push((message_id, msg));
        }
    }
    Ok(OpenCodeMessageQuery {
        messages,
        has_incomplete_usage,
    })
}

/// Parse a `message.data` JSON value into token fields. Returns `None` for an
/// all-zero message. OpenCode's self-reported `cost` is deliberately ignored —
/// cc one recomputes cost from its own pricing so the four-bucket split stays
/// consistent across parsers.
fn parse_opencode_message_data(value: &serde_json::Value) -> Option<OpenCodeMessageData> {
    let tokens = value.get("tokens")?;
    let n = |k: &str| tokens.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let input_tokens = n("input");
    let output_tokens = n("output");
    let reasoning_tokens = n("reasoning");
    let cache_obj = tokens.get("cache");
    let cache_read_tokens = cache_obj
        .and_then(|c| c.get("read"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let cache_write_tokens = cache_obj
        .and_then(|c| c.get("write"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    if input_tokens == 0
        && output_tokens == 0
        && reasoning_tokens == 0
        && cache_read_tokens == 0
        && cache_write_tokens == 0
    {
        return None;
    }
    let model_id = value
        .get("modelID")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let timestamp_ms = value
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    Some(OpenCodeMessageData {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        cache_write_tokens,
        model_id,
        timestamp_ms,
    })
}

fn opencode_raw_usage(session_id: &str, message_id: &str, msg: &OpenCodeMessageData) -> RawUsage {
    let parsed_ts = if msg.timestamp_ms > 0 {
        chrono::DateTime::from_timestamp_millis(msg.timestamp_ms).map(|dt| dt.to_rfc3339())
    } else {
        None
    };
    let timestamp = super::fallback_timestamp(parsed_ts);
    RawUsage {
        uuid: format!("opencode:{session_id}:{message_id}"),
        timestamp,
        model: msg.model_id.clone(),
        source: SOURCE_TAG.to_string(),
        session_id: session_id.to_string(),
        tokens: TokenCounts {
            input: msg.input_tokens,
            output: msg.output_tokens + msg.reasoning_tokens,
            cache_creation: msg.cache_write_tokens,
            cache_read: msg.cache_read_tokens,
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opencode_data_json(
        input: u32,
        output: u32,
        reasoning: u32,
        cache_read: u32,
        cache_write: u32,
        model: &str,
        completed: bool,
    ) -> String {
        let time = if completed {
            r#""time":{"created":1779755333700,"completed":1779755350639}"#.to_string()
        } else {
            r#""time":{"created":1779755333700}"#.to_string()
        };
        format!(
            r#"{{"role":"assistant","tokens":{{"input":{input},"output":{output},"reasoning":{reasoning},"cache":{{"read":{cache_read},"write":{cache_write}}}}},"modelID":"{model}",{time}}}"#
        )
    }

    #[test]
    fn opencode_parse_message_data_variants() {
        let full: serde_json::Value = serde_json::json!({
            "role": "assistant",
            "tokens": { "total": 56554, "input": 3272, "output": 383, "reasoning": 419,
                        "cache": { "write": 0, "read": 52480 } },
            "modelID": "deepseek-v4-pro",
            "providerID": "deepseek",
            "time": { "created": 1779755333700i64, "completed": 1779755350639i64 }
        });
        let d = parse_opencode_message_data(&full).unwrap();
        assert_eq!(d.input_tokens, 3272);
        assert_eq!(d.output_tokens, 383);
        assert_eq!(d.reasoning_tokens, 419);
        assert_eq!(d.cache_read_tokens, 52480);
        assert_eq!(d.cache_write_tokens, 0);
        assert_eq!(d.model_id, "deepseek-v4-pro");
        assert_eq!(d.timestamp_ms, 1779755333700);
        // missing cache ⇒ zeros.
        let no_cache: serde_json::Value = serde_json::json!({
            "role": "assistant", "tokens": { "input": 1000, "output": 200 },
            "modelID": "m", "time": { "created": 1, "completed": 2 }
        });
        let d = parse_opencode_message_data(&no_cache).unwrap();
        assert_eq!(d.cache_read_tokens, 0);
        assert_eq!(d.cache_write_tokens, 0);
        // all-zero ⇒ None.
        let zero: serde_json::Value = serde_json::json!({
            "role": "assistant",
            "tokens": { "input": 0, "output": 0, "reasoning": 0, "cache": { "read": 0, "write": 0 } },
            "modelID": "t", "time": { "created": 1, "completed": 2 }
        });
        assert!(parse_opencode_message_data(&zero).is_none());
    }

    #[test]
    fn opencode_query_skips_incomplete_messages() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);",
        )
        .unwrap();
        let done = opencode_data_json(1000, 200, 0, 0, 0, "m", true);
        let wip = opencode_data_json(500, 0, 0, 0, 0, "m", false);
        conn.execute(
            "INSERT INTO message VALUES ('done','s1',1,?1),('wip','s1',2,?2)",
            rusqlite::params![done, wip],
        )
        .unwrap();
        let qr = query_assistant_messages(&conn, "s1").unwrap();
        assert_eq!(qr.messages.len(), 1);
        assert_eq!(qr.messages[0].0, "done");
        assert!(qr.has_incomplete_usage);
    }

    #[test]
    fn opencode_query_sessions_uses_message_watermark() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT, time_updated INTEGER);
             CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
             INSERT INTO session VALUES ('s1', 100);
             INSERT INTO message VALUES ('m1', 's1', 90, 200, '{}');",
        )
        .unwrap();
        let sessions = query_sessions(&conn).unwrap();
        assert_eq!(sessions, vec![("s1".to_string(), 200)]);
    }

    #[test]
    fn opencode_parses_db_into_four_buckets() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE session (id TEXT, title TEXT, directory TEXT, time_created INTEGER, time_updated INTEGER);
                 CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
                 INSERT INTO session VALUES ('s1','','/p',0,100);
                 INSERT INTO message (id, session_id, time_created, time_updated) VALUES ('m1','s1',90,200);
                 INSERT INTO message (id, session_id, time_created, time_updated) VALUES ('m2','s1',91,201);",
            )
            .unwrap();
            conn.execute(
                "UPDATE message SET data = ?1 WHERE id = 'm1'",
                rusqlite::params![opencode_data_json(
                    3272,
                    383,
                    419,
                    52480,
                    0,
                    "deepseek-v4-pro",
                    true
                )],
            )
            .unwrap();
            conn.execute(
                "UPDATE message SET data = ?1 WHERE id = 'm2'",
                rusqlite::params![opencode_data_json(
                    10,
                    5,
                    0,
                    0,
                    100,
                    "anthropic/claude-opus-4-6",
                    true
                )],
            )
            .unwrap();
        }
        let p = OpenCodeSourceParser::with_db(db.clone());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.source, "opencode");
        assert_eq!(result.events.len(), 2);
        let by_id: std::collections::HashMap<&str, &RawUsage> =
            result.events.iter().map(|e| (e.uuid.as_str(), e)).collect();
        let m1 = by_id["opencode:s1:m1"];
        assert_eq!(m1.tokens.input, 3272);
        assert_eq!(
            m1.tokens.output, 802,
            "output folds in reasoning (383 + 419)"
        );
        assert_eq!(m1.tokens.cache_read, 52480);
        assert_eq!(m1.tokens.cache_creation, 0);
        assert_eq!(m1.model, "deepseek-v4-pro");
        let m2 = by_id["opencode:s1:m2"];
        assert_eq!(
            m2.tokens.cache_creation, 100,
            "cache.write maps to cache_creation"
        );
    }

    #[test]
    fn opencode_incremental_skips_already_synced_session() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE session (id TEXT, title TEXT, directory TEXT, time_created INTEGER, time_updated INTEGER);
                 CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
                 INSERT INTO session VALUES ('s1','','/p',0,100);
                 INSERT INTO message (id, session_id, time_created, time_updated) VALUES ('m1','s1',90,200);",
            )
            .unwrap();
            let data = opencode_data_json(3272, 383, 419, 52480, 0, "deepseek-v4-pro", true);
            conn.execute(
                "UPDATE message SET data = ?1 WHERE id = 'm1'",
                rusqlite::params![data],
            )
            .unwrap();
        }
        let p = OpenCodeSourceParser::with_db(db);
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.events.len(), 1);
        let progress: ScanProgress = delta;
        // Same db, no changes ⇒ file mtime gate skips it entirely.
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.events.len(), 0);
    }

    // ---- session + transcript extraction (OpenCode sessions) ----

    /// Build the full opencode schema (session/message/part) on an in-memory
    /// connection for the session/transcript tests below.
    fn opencode_schema(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                directory TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
             );
             CREATE TABLE message (
                id TEXT,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
             );
             CREATE TABLE part (
                id TEXT,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL
             );",
        )
        .unwrap();
    }

    /// RawSession fields + title fallback chain (title > directory basename >
    /// empty). Timestamps are ms-epoch → ISO8601.
    #[test]
    fn opencode_session_row_title_and_basename_fallback() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        opencode_schema(&conn);
        conn.execute_batch(
            "INSERT INTO session VALUES ('ses_a','My Session','/work/proj',1722500000000,1722600000000);
             INSERT INTO session VALUES ('ses_b','','/work/other',1722500000000,1722600000000);
             INSERT INTO session VALUES ('ses_c','','',1722500000000,1722600000000);",
        )
        .unwrap();
        let rows = query_session_rows(&conn).unwrap();
        assert_eq!(rows.len(), 3);

        // Title present → used verbatim; timestamps formatted ms-epoch → ISO.
        let a = opencode_raw_session(rows.iter().find(|r| r.id == "ses_a").unwrap());
        assert_eq!(a.id, "ses_a");
        assert_eq!(a.source, "opencode");
        assert_eq!(a.project_dir, "/work/proj");
        assert_eq!(a.title_orig, "My Session");
        assert_eq!(a.started_at, epoch_millis_to_iso(1_722_500_000_000));
        assert_eq!(a.last_active_at, epoch_millis_to_iso(1_722_600_000_000));

        // Empty title → directory basename.
        let b = opencode_raw_session(rows.iter().find(|r| r.id == "ses_b").unwrap());
        assert_eq!(b.title_orig, "other");
        assert_eq!(b.project_dir, "/work/other");

        // Empty title and directory → empty string (no panic).
        let c = opencode_raw_session(rows.iter().find(|r| r.id == "ses_c").unwrap());
        assert_eq!(c.title_orig, "");
    }

    /// Transcript join: one SessionMessage per message row, ordered by
    /// `time_created`; content = parts joined (text → text, tool → tool name);
    /// `name` carries the tool name; `uuid` is the stable message id; assistant
    /// model is preserved.
    #[test]
    fn opencode_transcript_joins_message_and_parts() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        opencode_schema(&conn);
        conn.execute_batch(
            "INSERT INTO session VALUES ('ses_1','T','/p',1000,4000);
             INSERT INTO message VALUES ('msg_u','ses_1',1000,1000,'{\"role\":\"user\"}');
             INSERT INTO message VALUES ('msg_a','ses_1',2000,2000,'{\"role\":\"assistant\",\"modelID\":\"glm-5.2\"}');",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO part VALUES ('p1','ses_1','msg_u',1100,'{\"type\":\"text\",\"text\":\"Hello\"}');
             INSERT INTO part VALUES ('p2','ses_1','msg_a',2100,'{\"type\":\"text\",\"text\":\"Hi there\"}');
             INSERT INTO part VALUES ('p3','ses_1','msg_a',2200,'{\"type\":\"tool\",\"tool\":\"bash\"}');",
        )
        .unwrap();
        let msgs = query_transcript_messages(&conn, "ses_1").unwrap();
        assert_eq!(msgs.len(), 2, "one SessionMessage per row, in time order");

        let u = &msgs[0];
        assert_eq!(u.uuid, "msg_u", "uuid is the stable message id");
        assert_eq!(u.session_id, "ses_1");
        assert_eq!(u.role, SessionMessageRole::User);
        assert_eq!(u.content, "Hello");
        assert!(u.model.is_none());
        assert!(u.name.is_none());

        let a = &msgs[1];
        assert_eq!(a.uuid, "msg_a");
        assert_eq!(a.role, SessionMessageRole::Assistant);
        assert_eq!(a.model.as_deref(), Some("glm-5.2"));
        // Content joins the text part and the tool part's tool name.
        assert_eq!(a.content, "Hi there\nbash");
        // Tool part surfaced its tool name on `name`.
        assert_eq!(a.name.as_deref(), Some("bash"));
    }

    /// Transcript skips messages with no parseable content (no parts) and
    /// messages whose role is outside {user, assistant, tool}.
    #[test]
    fn opencode_transcript_skips_empty_and_unknown_roles() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        opencode_schema(&conn);
        conn.execute_batch(
            "INSERT INTO session VALUES ('ses_1','T','/p',1000,4000);
             INSERT INTO message VALUES ('m_user','ses_1',1000,1000,'{\"role\":\"user\"}');
             INSERT INTO message VALUES ('m_empty','ses_1',2000,2000,'{\"role\":\"assistant\"}');
             INSERT INTO message VALUES ('m_weird','ses_1',3000,3000,'{\"role\":\"system\"}');",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO part VALUES ('p1','ses_1','m_user',1100,'{\"type\":\"text\",\"text\":\"hi\"}');
             INSERT INTO part VALUES ('p2','ses_1','m_weird',3100,'{\"type\":\"text\",\"text\":\"sys\"}');",
        )
        .unwrap();
        let msgs = query_transcript_messages(&conn, "ses_1").unwrap();
        // m_empty has no parts (empty content); m_weird has an unknown role.
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].uuid, "m_user");
    }

    /// End-to-end via `parse`: one RawSession with the DB title + ISO
    /// timestamps, transcript messages, and the usage event now carries
    /// `session_id` (was empty before this change).
    #[test]
    fn opencode_parse_emits_session_messages_and_usage_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            opencode_schema(&conn);
            conn.execute_batch(
                "INSERT INTO session VALUES ('ses_1','Build it','/home/me/vault',1722500000000,1722600000000);
                 INSERT INTO message VALUES ('msg_a','ses_1',1722550000000,1722550000000,'{\"role\":\"assistant\",\"modelID\":\"glm-5.2\"}');
                 INSERT INTO part VALUES ('p1','ses_1','msg_a',1722550001000,'{\"type\":\"text\",\"text\":\"drafting\"}');",
            )
            .unwrap();
            // A completed assistant usage message → RawUsage with session_id.
            let data = opencode_data_json(100, 20, 0, 0, 0, "glm-5.2", true);
            conn.execute(
                "INSERT INTO message VALUES ('msg_b','ses_1',1722550002000,1722550002000,?1)",
                rusqlite::params![data],
            )
            .unwrap();
        }
        let p = OpenCodeSourceParser::with_db(db.clone());
        let result = p.parse(&p.discover().unwrap()).unwrap();

        // RawSession: title from DB, ISO timestamps, source tagged opencode.
        assert_eq!(result.sessions.len(), 1);
        let s = &result.sessions[0];
        assert_eq!(s.id, "ses_1");
        assert_eq!(s.source, "opencode");
        assert_eq!(s.project_dir, "/home/me/vault");
        assert_eq!(s.title_orig, "Build it");
        assert_eq!(s.started_at, epoch_millis_to_iso(1_722_500_000_000));
        assert_eq!(s.last_active_at, epoch_millis_to_iso(1_722_600_000_000));

        // Usage event carries the session id (the field this task fills in).
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].session_id, "ses_1");

        // The assistant transcript message was extracted (msg_b has no parts ⇒
        // empty content ⇒ skipped, so only msg_a survives).
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].uuid, "msg_a");
        assert_eq!(result.messages[0].role, SessionMessageRole::Assistant);
        assert_eq!(result.messages[0].content, "drafting");
    }

    /// Incremental collect emits RawSession + transcript + session-stamped
    /// usage the first time a session advances.
    #[test]
    fn opencode_incremental_emits_session_and_messages_on_advance() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("opencode.db");
        {
            let conn = rusqlite::Connection::open(&db).unwrap();
            opencode_schema(&conn);
            conn.execute_batch("INSERT INTO session VALUES ('ses_1','T','/p',1000,1000);")
                .unwrap();
            let data = opencode_data_json(100, 20, 0, 0, 0, "glm-5.2", true);
            conn.execute(
                "INSERT INTO message VALUES ('msg_a','ses_1',1000,1000,?1)",
                rusqlite::params![data],
            )
            .unwrap();
            conn.execute_batch(
                "INSERT INTO part VALUES ('p1','ses_1','msg_a',1100,'{\"type\":\"text\",\"text\":\"hi\"}');",
            )
            .unwrap();
        }
        let p = OpenCodeSourceParser::with_db(db);
        let (r1, _delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        // First collect: session meta + transcript + session-stamped usage.
        assert_eq!(r1.sessions.len(), 1);
        assert_eq!(r1.sessions[0].id, "ses_1");
        assert_eq!(r1.messages.len(), 1);
        assert_eq!(r1.messages[0].uuid, "msg_a");
        assert_eq!(r1.events.len(), 1);
        assert_eq!(r1.events[0].session_id, "ses_1");
    }
}
