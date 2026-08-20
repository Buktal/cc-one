//! Codex (`~/.codex`) session-log parser.
//!
//! Reads `<codex_dir>/sessions/**/*.jsonl` (depth ≤ 3, i.e. `YYYY/MM/DD`) and
//! `<codex_dir>/archived_sessions/*.jsonl` (flat). Each line is one JSON event;
//! the parser consumes:
//!   - `session_meta` — session id + cwd (→ one [`RawSession`] per file);
//!   - `turn_context` — current model;
//!   - `event_msg` (subtype `token_count`) — cumulative usage → per-call delta;
//!   - `response_item` — transcript messages (user/assistant text + tool calls).
//!
//! Codex's `total_token_usage` is **cumulative** and its `input_tokens` is
//! cache-inclusive, so the parser computes per-call deltas and subtracts
//! `cache_read` to yield a fresh `input` (parse-time fresh-input
//! normalization). Sub-agent / fork logs replay the parent thread's history
//! before their own usage; that replay only re-establishes the cumulative
//! baseline and is never emitted, and such sessions produce no [`RawSession`] /
//! transcript (they are not user-facing top-level sessions).

use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::model::{RawSession, SessionMessage, SessionMessageRole, TokenCounts};

use super::{
    collect_jsonl_incremental, discover_files, is_jsonl_file, normalize_cache_inclusive, truncate,
    CollectResult, DirectoryShape, FileParseOutcome, RawUsage, ScanProgress, ScanProgressDelta,
    SourceParser, TITLE_MAX, TRIM_LIMIT,
};

/// Stable source tag — becomes `RawUsage.source` / `RawSession.source` and the
/// DB source column; the single literal behind `name()`, usage, and session
/// construction.
const SOURCE_TAG: &str = "codex_cli";

/// Codex (`~/.codex`) session-log parser.
///
/// Reads `<codex_dir>/sessions/**/*.jsonl` (depth ≤ 3, i.e. `YYYY/MM/DD`) and
/// `<codex_dir>/archived_sessions/*.jsonl` (flat). Only `session_meta`,
/// `turn_context`, and `event_msg` (subtype `token_count`) events are consumed.
///
/// Codex's `total_token_usage` is **cumulative** and its `input_tokens` is
/// cache-inclusive, so the parser computes per-call deltas and subtracts
/// `cache_read` to yield a fresh `input` (parse-time fresh-input
/// normalization). Sub-agent / fork logs replay the parent thread's history
/// before their own usage; that replay only re-establishes the cumulative
/// baseline and is never emitted.
pub struct CodexSourceParser {
    codex_dir: PathBuf,
}

impl CodexSourceParser {
    /// Root-injection seam: parser rooted at `home/.codex`. The collect
    /// orchestration factory (`all_source_parsers_at`) builds every parser
    /// through this seam, so tests can point the whole chain at a tempdir
    /// fixture instead of the real `~`.
    pub(crate) fn new_at(home: &Path) -> Self {
        Self {
            codex_dir: home.join(".codex"),
        }
    }

    /// Test/override constructor with an explicit codex dir.
    #[cfg(test)]
    pub(crate) fn with_dir(dir: PathBuf) -> Self {
        Self { codex_dir: dir }
    }
}

impl SourceParser for CodexSourceParser {
    fn name(&self) -> &'static str {
        SOURCE_TAG
    }

    fn discover(&self) -> AppResult<Vec<PathBuf>> {
        // Two shapes: `sessions/**` (three dir levels deep, i.e. `YYYY/MM/DD`)
        // and flat `archived_sessions/` (top level only). A missing codex dir
        // is not an error — the shared skeleton yields no files for absent
        // roots.
        Ok(discover_files(
            &[
                DirectoryShape {
                    root: self.codex_dir.join("sessions"),
                    max_depth: Some(4), // sessions/YYYY/MM/DD/*.jsonl
                },
                DirectoryShape {
                    root: self.codex_dir.join("archived_sessions"),
                    max_depth: Some(1), // flat: files directly in the dir
                },
            ],
            is_jsonl_file,
        ))
    }

    fn parse(&self, files: &[PathBuf]) -> AppResult<CollectResult> {
        super::parse_jsonl_full(self, files, fold_codex_file)
    }

    /// Incremental collect: mtime-gate unchanged files; for a changed file,
    /// re-parse it fully to rebuild the cumulative baseline + event_index, but
    /// only EMIT events/messages past the recorded cursor. Session meta is
    /// rebuilt from the whole file every pass (refreshable system data). The
    /// baseline cannot be cached (it depends on every prior line), so old lines
    /// are still parsed — the saving is skipping unchanged files entirely + not
    /// re-emitting seen rows. Both `parse` and this path share `fold_codex_file`,
    /// so test and production run identical logic.
    fn collect_incremental(
        &self,
        progress: &ScanProgress,
    ) -> AppResult<(CollectResult, ScanProgressDelta)> {
        collect_jsonl_incremental(self, progress, |file: &Path, text, start_line| {
            fold_codex_file(file, text, start_line)
        })
    }

    /// Codex session ids are NOT file stems — the thread id lives in the file's
    /// `session_meta` (with the rollout-filename UUID as fallback). Must read
    /// the head to reconcile: the stem default would mis-delete real sessions.
    fn session_ids_seen(&self, files: &[std::path::PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|f| {
                // 有界头读（session_meta 在文件顶部）；失败回退文件名解析——
                // 回退只会多保留一行、绝不误删真实会话（见 read_head_utf8）。
                let head = super::read_head_utf8(f);
                let identity = prescan_codex_text(&head).0;
                resolve_session_id(f, identity.as_ref())
            })
            .collect()
    }
}

// ---- Codex parsing internals (pure, ported from CC-Switch's scanner) ----

/// Cumulative token usage tracked across a file (the `total_token_usage` field).
#[derive(Debug, Clone, Default)]
struct CumulativeTokens {
    input: u64,
    cached_input: u64,
    output: u64,
}

/// Per-call delta derived from two cumulative snapshots.
#[derive(Debug)]
struct DeltaTokens {
    input: u32,
    cached_input: u32,
    output: u32,
}

impl DeltaTokens {
    fn is_zero(&self) -> bool {
        self.input == 0 && self.cached_input == 0 && self.output == 0
    }
}

/// Per-file parse state advanced line by line.
struct CodexFileState {
    thread_id: Option<String>,
    current_model: String,
    prev_total: Option<CumulativeTokens>,
    event_index: u32,
    /// Events built before the model was known (deferred — see `learn_model`).
    pending_unknown: Vec<RawUsage>,
    /// Rows an earlier pass already wrote as "unknown" (correction candidates —
    /// see `learn_model`).
    stale_unknown: Vec<RawUsage>,
    /// Whether this pass has learned the model yet (guards `learn_model`'s
    /// one-shot flush against a later model switch re-flushing).
    model_resolved: bool,
}

/// A Codex session's identity: its unique thread id + whether it carries a
/// replayed parent-thread history snapshot (sub-agent or fork).
#[derive(Debug, Clone, PartialEq, Eq)]
struct CodexSessionIdentity {
    thread_id: String,
    carries_history_snapshot: bool,
}

/// One pre-scan pass over the file text: recover the session identity (first
/// `session_meta`) and — only if that session carries a history snapshot — the
/// 1-based line number of the first takeover event (`thread_settings_applied`
/// or `inter_agent_communication*`), before which token events are replay.
fn prescan_codex_text(text: &str) -> (Option<CodexSessionIdentity>, Option<i64>) {
    let mut identity = None;
    let mut boundary = None;
    for (index, line) in text.lines().enumerate() {
        if identity.is_none() && line.contains("\"session_meta\"") {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
                    if let Some(id) = value.get("payload").and_then(parse_codex_session_identity) {
                        identity = Some(id);
                    }
                }
            }
        }
        if boundary.is_none()
            && (line.contains("\"thread_settings_applied\"")
                || line.contains("\"inter_agent_communication"))
        {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(event_type) = value.get("type").and_then(|v| v.as_str()) {
                    let is_boundary = event_type.starts_with("inter_agent_communication")
                        || (event_type == "event_msg"
                            && value
                                .get("payload")
                                .and_then(|p| p.get("type"))
                                .and_then(|v| v.as_str())
                                == Some("thread_settings_applied"));
                    if is_boundary {
                        boundary = Some(index as i64 + 1);
                    }
                }
            }
        }
    }
    let boundary = identity.as_ref().and_then(|id| {
        if id.carries_history_snapshot {
            boundary
        } else {
            None
        }
    });
    (identity, boundary)
}

/// Fold one Codex JSONL file's text into a per-file parse outcome. Mirrors
/// claude's `fold_file`: three streams from a single forward pass —
///   - per-call usages (cumulative → delta; only lines past `start_line`);
///   - one [`RawSession`] covering the WHOLE file (system data is refreshable,
///     so every pass re-reads first/last ts, cwd, and the title sources);
///   - transcript [`SessionMessage`]s (only lines past `start_line` — incremental,
///     so a re-collect appends only new lines).
///
/// `start_line` is the 1-based cursor: lines at or before it rebuild state but
/// are not re-emitted as usages/messages (0 ⇒ emit all). Session meta is always
/// rebuilt from the full file.
///
/// The model context can lag the token events (Codex writes `turn_context`
/// only when the model is resolved; early token_count `info` carries no
/// model). Events built while the model is unknown are deferred — see
/// [`learn_model`] — so they get the model once it appears instead of being
/// permanently stamped "unknown". The two deferral lists map onto the
/// [`CollectResult`] channels: events THIS pass builds past the cursor become
/// regular `events`; events an EARLIER pass already wrote (at/before the
/// cursor, `model="unknown"` in the store) become `corrections` — re-emitted
/// with their original uuids for the store's guarded upsert.
///
/// Sub-agent / fork sessions (`source.subagent` / `forked_from_id` / session id
/// ≠ thread id) emit no [`RawSession`] and no transcript — only their own
/// post-boundary usage (with an empty `session_id`, since they have no
/// top-level session row to group under).
fn fold_codex_file(file: &Path, text: &str, start_line: i64) -> FileParseOutcome {
    let (identity, boundary) = prescan_codex_text(text);
    let is_subagent = identity
        .as_ref()
        .is_some_and(|i| i.carries_history_snapshot);
    let session_id = resolve_session_id(file, identity.as_ref());

    let mut state = CodexFileState {
        thread_id: identity.map(|i| i.thread_id),
        current_model: "unknown".to_string(),
        prev_total: None,
        event_index: 0,
        pending_unknown: Vec::new(),
        stale_unknown: Vec::new(),
        model_resolved: false,
    };
    // Sub-agent usage keeps an empty session_id — no top-level session exists.
    let session_id_for_usage = if is_subagent {
        String::new()
    } else {
        session_id.clone()
    };

    let mut events = Vec::new();
    let mut corrections = Vec::new();
    let mut messages = Vec::new();
    let mut skipped = 0u32;

    // Session meta — tracked over the FULL file (refreshable system data).
    let mut started_at = String::new();
    let mut last_active_at = String::new();
    let mut project_dir = String::new();
    let mut first_user_text: Option<String> = None;
    let mut saw_any_event = false;

    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx as i64 + 1; // 1-based, matching the cursor
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // Cheap substring gate before serde.
        let is_session_meta = line.contains("\"session_meta\"");
        let is_turn_context = line.contains("\"turn_context\"");
        let is_event_msg = line.contains("\"event_msg\"");
        let is_response_item = line.contains("\"response_item\"");
        if !is_session_meta && !is_turn_context && !is_event_msg && !is_response_item {
            continue;
        }
        // event_msg: only token_count carries usage; other subtypes are noise.
        if is_event_msg && !line.contains("\"token_count\"") {
            continue;
        }

        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(event_type) = value.get("type").and_then(|t| t.as_str()) else {
            continue;
        };

        // ---- session meta (full file, every pass) ----
        let ts = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !saw_any_event {
            started_at = ts.to_string();
            saw_any_event = true;
        }
        if !ts.is_empty() {
            last_active_at = ts.to_string();
        }
        // Title candidate: first real user message (injection-noise-filtered).
        // Skipped for sub-agents — they emit no session anyway.
        if first_user_text.is_none() && is_response_item && !is_subagent {
            if let Some(payload) = value.get("payload") {
                if payload.get("type").and_then(|t| t.as_str()) == Some("message")
                    && payload.get("role").and_then(|r| r.as_str()) == Some("user")
                {
                    if let Some(content) = payload.get("content") {
                        let text = extract_codex_message_text(content);
                        if let Some(candidate) = title_candidate_from_user_message(&text) {
                            first_user_text = Some(candidate);
                        }
                    }
                }
            }
        }

        match event_type {
            "session_meta" => {
                if let Some(payload) = value.get("payload") {
                    if state.thread_id.is_none() {
                        state.thread_id =
                            parse_codex_session_identity(payload).map(|i| i.thread_id);
                    }
                    if project_dir.is_empty() {
                        if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                            if !cwd.is_empty() {
                                project_dir = cwd.to_string();
                            }
                        }
                    }
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    if let Some(model) = payload
                        .get("model")
                        .or_else(|| payload.get("info").and_then(|i| i.get("model")))
                        .and_then(|v| v.as_str())
                    {
                        learn_model(model, &mut state, &mut events, &mut corrections);
                    }
                }
            }
            "event_msg" => {
                let Some(payload) = value.get("payload") else {
                    continue;
                };
                if payload.get("type").and_then(|t| t.as_str()) != Some("token_count") {
                    continue;
                }
                let info = match payload.get("info") {
                    Some(i) if !i.is_null() => i,
                    _ => continue, // first event often has null info
                };
                if let Some(model) = info
                    .get("model")
                    .or_else(|| info.get("model_name"))
                    .or_else(|| payload.get("model"))
                    .and_then(|v| v.as_str())
                {
                    learn_model(model, &mut state, &mut events, &mut corrections);
                }
                // Prefer cumulative total_token_usage; fall back to last_token_usage
                // (already a per-call delta).
                let (cumulative, is_total) = if let Some(total) = info.get("total_token_usage") {
                    (parse_cumulative_tokens(total), true)
                } else if let Some(last) = info.get("last_token_usage") {
                    (parse_cumulative_tokens(last), false)
                } else {
                    continue;
                };
                let Some(cumulative) = cumulative else {
                    continue;
                };
                let mut delta = if is_total {
                    let d = compute_delta(&state.prev_total, &cumulative);
                    state.prev_total = Some(cumulative);
                    d
                } else {
                    DeltaTokens {
                        input: cumulative.input as u32,
                        cached_input: cumulative.cached_input as u32,
                        output: cumulative.output as u32,
                    }
                };
                // Clamp before the zero-gate below: an abnormal delta (input 0,
                // cached > 0) must read as zero so it is skipped. The shared
                // normalizer re-clamps (idempotently) when building the event.
                delta.cached_input = delta.cached_input.min(delta.input);
                if delta.is_zero() {
                    continue; // task-boundary snapshot, no new usage
                }
                // Every non-zero event occupies a stable sequence number — line
                // numbers drift if the file is edited, this does not.
                state.event_index += 1;

                // History replay only re-establishes the baseline — never emit.
                if is_history_snapshot_event(boundary, line_no) {
                    if line_no > start_line {
                        skipped += 1;
                    }
                    continue;
                }
                let thread_id = state.thread_id.as_deref().unwrap_or("unknown");
                let timestamp = value
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                // Codex input is cache-inclusive — normalize to fresh via the
                // shared helper (clamp already applied above for the zero-gate).
                let (fresh_input, clamped_cache_read) =
                    normalize_cache_inclusive(delta.input, delta.cached_input);
                let usage = RawUsage {
                    uuid: format!("codex:thread-v1:{thread_id}:{}", state.event_index),
                    timestamp: super::fallback_timestamp(timestamp.clone()),
                    model: state.current_model.clone(),
                    source: SOURCE_TAG.to_string(),
                    session_id: session_id_for_usage.clone(),
                    tokens: TokenCounts {
                        input: fresh_input,
                        output: delta.output,
                        cache_creation: 0,
                        cache_read: clamped_cache_read,
                    },
                    ..Default::default()
                };
                // Model resolution lags the token events (see `learn_model`),
                // so route each built event accordingly:
                //   - already-synced lines (≤ cursor) rebuild state but are not
                //     re-emitted — unless their model was "unknown", in which
                //     case they become CORRECTION CANDIDATES for the pass that
                //     first sees the model (`learn_model` flushes them into the
                //     `corrections` output channel, not `events` — the store
                //     rewrites only rows that still read model='unknown');
                //   - lines past the cursor emit now when the model is known,
                //     and are deferred otherwise (flushed by `learn_model`, or
                //     with the "unknown" fallback at EOF).
                if line_no <= start_line {
                    if state.current_model == "unknown" {
                        state.stale_unknown.push(usage);
                    }
                    continue;
                }
                if state.current_model == "unknown" {
                    state.pending_unknown.push(usage);
                } else {
                    events.push(usage);
                }
            }
            // Transcript messages — only past the cursor, only for top-level
            // sessions (sub-agent transcripts are dropped). The guard collapses
            // the cursor/sub-agent gate into the arm so already-synced and
            // sub-agent response_items fall straight through to `_`.
            "response_item" if line_no > start_line && !is_subagent => {
                if let Some(payload) = value.get("payload") {
                    messages.extend(extract_codex_messages(payload, &session_id, ts, line_no));
                }
            }
            _ => {}
        }
    }

    // The file carried no model context this pass — this-pass deferred events
    // fall back to "unknown" (unchanged from pre-fix behavior; the pass that
    // later sees the model re-emits them as corrections via `learn_model`).
    // Stale candidates are dropped: they were already written by an earlier
    // pass as "unknown" and will be re-collected + re-offered as corrections
    // on the pass that sees the model (or a later one).
    events.extend(std::mem::take(&mut state.pending_unknown));

    let sessions = if !is_subagent && saw_any_event {
        // Title priority: first real user message (noise-filtered) → cwd basename.
        // TODO: state_5.sqlite `threads.title` is a richer title source (CC-Switch
        // loads it via `load_thread_titles_from_db`), but locating/reading the
        // Codex state DB needs its own discovery path; until that lands, the
        // first real prompt is the best-effort title (cwd basename fallback).
        let title_orig = first_user_text
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                Path::new(&project_dir)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|s| !s.is_empty())
            });
        let title_orig = truncate(title_orig.unwrap_or(""), TITLE_MAX);
        vec![RawSession {
            id: session_id,
            source: SOURCE_TAG.to_string(),
            project_dir,
            title_orig,
            started_at,
            last_active_at,
            agent_type: String::new(),
            parent_session_id: String::new(),
        }]
    } else {
        Vec::new()
    };

    FileParseOutcome {
        events,
        corrections,
        turn_durations: Vec::new(),
        sessions,
        messages,
        skipped,
    }
}

fn is_history_snapshot_event(boundary: Option<i64>, line_offset: i64) -> bool {
    boundary.is_some_and(|b| line_offset < b)
}

/// Learn the current model from a `turn_context` / token_count `info` line and
/// resolve the events that had to wait for it.
///
/// Codex writes the model context AFTER the first token events (it is resolved
/// per turn; early token_count `info` carries no model), so events built before
/// it must not be permanently stamped "unknown". Two deferral lists on `state`
/// are flushed here onto the two [`CollectResult`] channels:
///   - `pending_unknown` → `events` — rows emitted THIS pass before the model
///     line; never written before, they are ordinary new events once resolved;
///   - `stale_unknown` → `corrections` — rows at or before the cursor, written
///     by an EARLIER pass as "unknown"; re-emitted with their ORIGINAL uuids so
///     the ingest layer's guarded upsert rewrites exactly those store rows
///     (the protocol's store half, `Store::ingest_corrections_marking_dirty`).
///
/// The stale re-emission runs in EVERY pass that sees the model, cursor or
/// not: the parser cannot tell which pre-model rows an earlier pass wrote
/// before the fix — re-offering costs a few deduped rows per file, and the
/// store's `WHERE model = 'unknown'` guard makes the rewrite a no-op for rows
/// that already carry the model. Subsequent model switches in the same pass
/// just update `current_model`; later events use it directly.
fn learn_model(
    model: &str,
    state: &mut CodexFileState,
    events: &mut Vec<RawUsage>,
    corrections: &mut Vec<RawUsage>,
) {
    state.current_model = crate::model::normalize_model_key(model);
    if state.model_resolved {
        return;
    }
    state.model_resolved = true;
    let model = state.current_model.clone();
    for usage in state.stale_unknown.drain(..) {
        corrections.push(RawUsage {
            model: model.clone(),
            ..usage
        });
    }
    for usage in state.pending_unknown.drain(..) {
        events.push(RawUsage {
            model: model.clone(),
            ..usage
        });
    }
}

// ---- Session id resolution (session_meta id → filename UUID fallback) ----

/// Resolve the session id: prefer `session_meta.payload.id`, fall back to the
/// UUID embedded in the rollout filename (`rollout-<ts>-<uuid>.jsonl`), then
/// the file stem. CC-Switch does the same via a UUID regex; we validate the
/// trailing `8-4-4-4-12` hex shape by hand to avoid pulling in the regex crate.
fn resolve_session_id(file: &Path, identity: Option<&CodexSessionIdentity>) -> String {
    if let Some(id) = identity
        .map(|i| i.thread_id.as_str())
        .filter(|s| !s.is_empty())
    {
        return id.to_string();
    }
    infer_session_id_from_filename(file).unwrap_or_else(|| {
        file.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    })
}

/// Extract a trailing UUID (`8-4-4-4-12` hex) from a filename's stem, e.g.
/// `rollout-2026-03-06T21-50-12-019cc369-bd7c-7891-b371-7b20b4fe0b18` → the
/// UUID. Returns None when the stem does not end in a well-formed UUID.
fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(".jsonl").unwrap_or(name);
    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < 36 {
        return None;
    }
    let tail: String = chars[chars.len() - 36..].iter().collect();
    if is_uuid_format(&tail) {
        Some(tail)
    } else {
        None
    }
}

/// Validate the `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` UUID shape (ASCII hex
/// digits at every non-dash position). UUIDs are pure ASCII, so byte indexing
/// on the candidate string is safe.
fn is_uuid_format(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && [8usize, 13, 18, 23].iter().all(|&i| bytes[i] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 8 | 13 | 18 | 23) || b.is_ascii_hexdigit())
}

// ---- Transcript message extraction ----
//
// Codex `response_item` events carry the transcript. Mapping to cc one's four
// roles: `message` with role user/assistant → User/Assistant (developer is
// injected instructions, dropped); `function_call` → Tool with the tool name.
// `function_call_output` is dropped — the function_call line already records
// the call, and outputs are often verbose (claude.rs drops user-role
// tool_results for the same reason: keep the transcript lean).

/// Extract transcript message lines from one `response_item` payload. Each
/// emitted line gets a stable uuid — the payload's `id` when present, else
/// `session_id:L<line_no>` — so re-collects append idempotently.
fn extract_codex_messages(
    payload: &serde_json::Value,
    session_id: &str,
    ts: &str,
    line_no: i64,
) -> Vec<SessionMessage> {
    let payload_type = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let uuid = payload
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{session_id}:L{line_no}"));
    let mk =
        |role: SessionMessageRole, model: Option<String>, name: Option<String>, content: String| {
            SessionMessage {
                uuid: uuid.clone(),
                session_id: session_id.to_string(),
                role,
                ts: ts.to_string(),
                model,
                name,
                content,
            }
        };

    let mut out = Vec::new();
    match payload_type {
        "message" => {
            let role = payload.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let mapped = match role {
                "user" => Some(SessionMessageRole::User),
                "assistant" => Some(SessionMessageRole::Assistant),
                // developer messages are injected instructions (e.g. permissions
                // preamble); they are not user dialog, so drop them.
                _ => None,
            };
            if let Some(role) = mapped {
                if let Some(content) = payload.get("content") {
                    let text = truncate(&extract_codex_message_text(content), TRIM_LIMIT);
                    if !text.is_empty() {
                        out.push(mk(role, None, None, text));
                    }
                }
            }
        }
        "function_call" => {
            let name = payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("tool")
                .to_string();
            // Arguments are a JSON string; keep a trimmed copy (lean, like
            // claude's tool_use input cap).
            let args = payload
                .get("arguments")
                .and_then(|v| v.as_str())
                .map(|s| truncate(s, 1024))
                .unwrap_or_default();
            out.push(mk(SessionMessageRole::Tool, None, Some(name), args));
        }
        // function_call_output / unknown payload types → drop (see note above).
        _ => {}
    }
    out
}

/// Flatten a Codex message `content` field into plain text. The value is either
/// a string (plain user text) or an array of items whose text lives under one
/// of the `text` / `input_text` / `output_text` keys (Codex's content-block
/// shape, distinct from Claude's `{"type":"text","text":…}` arrays).
fn extract_codex_message_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                ["text", "input_text", "output_text"]
                    .iter()
                    .find_map(|key| item.get(key).and_then(|v| v.as_str()))
                    .map(str::to_string)
                    .filter(|s| !s.trim().is_empty())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

// ---- Original-title extraction (ported from CC-Switch) ----
//
// Codex has no `summary`/`customTitle` event; the original title is the first
// real user message, with two injected-noise shapes skipped so the title is the
// actual prompt:
//   - `# AGENTS.md` preamble and `<environment_context>` blocks;
//   - VS Code's `# Context from my IDE setup:` wrapper — the real prompt lives
//     in its LAST `## My request for Codex:` section.

/// VS Code IDE-context wrapper prefix; the real prompt is nested inside.
const VSCODE_CONTEXT_PREFIX: &str = "# Context from my IDE setup:";
/// Lowercased heading marker for the inline IDE-request section.
const CODEX_REQUEST_MARKER: &str = "my request for codex";

/// Decide whether a user message is a usable title candidate, and extract the
/// real prompt from a VS Code IDE-context wrapper when present. Returns None
/// for known injection noise or an IDE-context block with no request section
/// (so the caller keeps scanning for the next user message).
fn title_candidate_from_user_message(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("# AGENTS.md")
        || trimmed.starts_with("<environment_context>")
    {
        return None;
    }
    if trimmed.starts_with(VSCODE_CONTEXT_PREFIX) {
        return extract_codex_prompt_from_ide_context(trimmed);
    }
    Some(trimmed.to_string())
}

/// Extract the real prompt from a VS Code IDE-context block: the body of the
/// LAST `## My request for Codex:` heading. Earlier matches can be headings
/// inside the active selection / open file content. Ported from CC-Switch
/// (which documents this best-effort trade-off in its tests).
fn extract_codex_prompt_from_ide_context(text: &str) -> Option<String> {
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let mut prompt: Option<String> = None;
    for (index, line) in lines.iter().enumerate() {
        let Some(inline_prompt) = codex_request_heading_payload(line) else {
            continue;
        };
        if !inline_prompt.is_empty() {
            prompt = Some(inline_prompt.to_string());
            continue;
        }
        let following = lines[index + 1..].join("\n").trim().to_string();
        prompt = (!following.is_empty()).then_some(following);
    }
    prompt
}

/// If `line` is a `## My request for Codex[:…]` heading, return the inline text
/// after the separator (or `""` when the prompt is on the following lines).
fn codex_request_heading_payload(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let heading = trimmed.trim_start_matches('#').trim_start();
    let lowered = heading.to_ascii_lowercase();
    if !lowered.starts_with(CODEX_REQUEST_MARKER) {
        return None;
    }
    // CODEX_REQUEST_MARKER is ASCII, so byte indexing into `heading` is safe.
    let suffix = heading[CODEX_REQUEST_MARKER.len()..].trim_start();
    if suffix.is_empty() {
        return Some("");
    }
    let Some(separator) = suffix.chars().next() else {
        return Some("");
    };
    if !matches!(separator, ':' | '：' | '-' | '—') {
        return None;
    }
    Some(
        suffix
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '：' | '-' | '—'))
            .trim(),
    )
}

/// Extract the session identity from a `session_meta` payload. The `id` is the
/// unique thread id; `session_id` points at the parent thread for sub-agents.
fn parse_codex_session_identity(payload: &serde_json::Value) -> Option<CodexSessionIdentity> {
    let thread_id = payload
        .get("id")
        .or_else(|| payload.get("thread_id"))
        .or_else(|| payload.get("threadId"))
        .or_else(|| payload.get("session_id"))
        .or_else(|| payload.get("sessionId"))
        .and_then(|v| v.as_str())?
        .to_string();
    let session_id = payload
        .get("session_id")
        .or_else(|| payload.get("sessionId"))
        .and_then(|v| v.as_str());
    let carries_history_snapshot = payload
        .get("forked_from_id")
        .and_then(|v| v.as_str())
        .is_some_and(|v| !v.is_empty())
        || payload
            .get("source")
            .and_then(|s| s.get("subagent"))
            .is_some()
        || session_id.is_some_and(|sid| sid != thread_id);
    Some(CodexSessionIdentity {
        thread_id,
        carries_history_snapshot,
    })
}

/// Delta between two cumulative snapshots (saturating to guard against the
/// current falling below the previous — abnormal but non-fatal).
fn compute_delta(prev: &Option<CumulativeTokens>, current: &CumulativeTokens) -> DeltaTokens {
    match prev {
        None => DeltaTokens {
            input: current.input as u32,
            cached_input: current.cached_input as u32,
            output: current.output as u32,
        },
        Some(p) => DeltaTokens {
            input: current.input.saturating_sub(p.input) as u32,
            cached_input: current.cached_input.saturating_sub(p.cached_input) as u32,
            output: current.output.saturating_sub(p.output) as u32,
        },
    }
}

/// Extract cumulative tokens from a `total_token_usage` / `last_token_usage`
/// object. `cached_input_tokens` and `cache_read_input_tokens` are both
/// accepted (field name varies across Codex versions).
fn parse_cumulative_tokens(total_usage: &serde_json::Value) -> Option<CumulativeTokens> {
    if total_usage.is_null() || !total_usage.is_object() {
        return None;
    }
    Some(CumulativeTokens {
        input: total_usage
            .get("input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cached_input: total_usage
            .get("cached_input_tokens")
            .or_else(|| total_usage.get("cache_read_input_tokens"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output: total_usage
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_jsonl(path: &Path, values: &[serde_json::Value]) {
        let contents = values
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(path, contents).unwrap();
    }

    fn codex_session_meta(thread_id: &str, session_id: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-07-10T03:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": thread_id,
                "session_id": session_id,
                "source": if thread_id == session_id {
                    serde_json::Value::String("cli".to_string())
                } else {
                    serde_json::json!({ "subagent": {} })
                }
            }
        })
    }

    fn codex_turn_context(model: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-07-10T03:00:01Z",
            "type": "turn_context",
            "payload": { "model": model }
        })
    }

    fn codex_token_count(input: u64, cached: u64, output: u64) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-07-10T03:00:02Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": { "total_token_usage": {
                    "input_tokens": input,
                    "cached_input_tokens": cached,
                    "output_tokens": output
                }}
            }
        })
    }

    /// Codex model names flow through `crate::model::normalize_model_key` (the
    /// shared superset normalizer). This pins the parser's observable
    /// normalization contract: lowercase, strip `provider/` prefix, and strip
    /// `-YYYY-MM-DD` / `-YYYYMMDD` date suffixes.
    #[test]
    fn codex_normalize_model_lowercase_prefix_and_dates() {
        let norm = crate::model::normalize_model_key;
        assert_eq!(norm("GLM-4.6"), "glm-4.6");
        assert_eq!(norm("openai/gpt-5.4"), "gpt-5.4");
        assert_eq!(norm("OPENAI/GPT-5.4"), "gpt-5.4");
        assert_eq!(norm("gpt-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(norm("gpt-5.4-pro-2026-03-05"), "gpt-5.4-pro");
        assert_eq!(norm("gpt-5.4-20260305"), "gpt-5.4");
        assert_eq!(norm("claude-opus-4-6-20260206"), "claude-opus-4-6");
        assert_eq!(norm("openai/GPT-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(norm("openai/gpt-5.4-20260305"), "gpt-5.4");
        assert_eq!(norm("gpt-5.2-codex"), "gpt-5.2-codex");
        assert_eq!(norm("o3"), "o3");
    }

    #[test]
    fn codex_compute_delta_first_subsequent_zero_saturating() {
        let first = compute_delta(
            &None,
            &CumulativeTokens {
                input: 17934,
                cached_input: 9600,
                output: 454,
            },
        );
        assert_eq!(first.input, 17934);
        assert_eq!(first.cached_input, 9600);
        assert_eq!(first.output, 454);
        let next = compute_delta(
            &Some(CumulativeTokens {
                input: 17934,
                cached_input: 9600,
                output: 454,
            }),
            &CumulativeTokens {
                input: 36722,
                cached_input: 27904,
                output: 804,
            },
        );
        assert_eq!(next.input, 36722 - 17934);
        assert_eq!(next.cached_input, 27904 - 9600);
        assert_eq!(next.output, 804 - 454);
        // task boundary: identical cumulative ⇒ zero delta.
        let zero = compute_delta(
            &Some(CumulativeTokens {
                input: 58346,
                cached_input: 46976,
                output: 1045,
            }),
            &CumulativeTokens {
                input: 58346,
                cached_input: 46976,
                output: 1045,
            },
        );
        assert!(zero.is_zero());
        // abnormal: current < previous ⇒ saturates to zero.
        let sat = compute_delta(
            &Some(CumulativeTokens {
                input: 100,
                cached_input: 50,
                output: 30,
            }),
            &CumulativeTokens {
                input: 80,
                cached_input: 40,
                output: 20,
            },
        );
        assert!(sat.is_zero());
    }

    #[test]
    fn codex_parse_cumulative_tokens_variants() {
        let v: serde_json::Value = serde_json::json!({
            "input_tokens": 17934, "cached_input_tokens": 9600, "output_tokens": 454,
            "reasoning_output_tokens": 233, "total_tokens": 18388
        });
        let t = parse_cumulative_tokens(&v).unwrap();
        assert_eq!(t.input, 17934);
        assert_eq!(t.cached_input, 9600);
        assert_eq!(t.output, 454);
        assert!(parse_cumulative_tokens(&serde_json::Value::Null).is_none());
        // alt field name cache_read_input_tokens.
        let alt: serde_json::Value = serde_json::json!({
            "input_tokens": 1000, "cache_read_input_tokens": 500, "output_tokens": 200
        });
        assert_eq!(parse_cumulative_tokens(&alt).unwrap().cached_input, 500);
    }

    #[test]
    fn codex_cached_clamped_to_input() {
        let prev = Some(CumulativeTokens {
            input: 100,
            cached_input: 0,
            output: 50,
        });
        let current = CumulativeTokens {
            input: 110,
            cached_input: 80,
            output: 60,
        };
        let mut delta = compute_delta(&prev, &current);
        // before clamp: input delta 10, cached delta 80 (abnormal, > input)
        assert_eq!(delta.input, 10);
        assert_eq!(delta.cached_input, 80);
        delta.cached_input = delta.cached_input.min(delta.input);
        assert_eq!(delta.cached_input, 10);
    }

    #[test]
    fn codex_discover_missing_dir_returns_empty() {
        let base = tempfile::tempdir().unwrap();
        let p = CodexSourceParser::with_dir(base.path().join("nope"));
        assert!(p.discover().unwrap().is_empty());
    }

    #[test]
    fn codex_subagent_identity_prefers_unique_thread_id() {
        let id = parse_codex_session_identity(
            codex_session_meta("child", "parent")
                .get("payload")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(id.thread_id, "child");
        assert!(id.carries_history_snapshot);
    }

    /// CC-Switch's `test_subagent_replay_only_establishes_token_baseline`:
    /// the replayed history (lines before `thread_settings_applied`) only sets
    /// the cumulative baseline; the child's own usage is the post-boundary delta.
    /// CC-Switch stores input=100 (cache-inclusive); cc one normalizes to
    /// fresh at parse ⇒ input = 100 − 50 = 50 (the documented Codex divergence).
    #[test]
    fn codex_subagent_replay_emits_only_child_usage_with_fresh_input() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir
            .path()
            .join("sessions")
            .join("2026")
            .join("07")
            .join("child.jsonl");
        std::fs::create_dir_all(child.parent().unwrap()).unwrap();
        write_jsonl(
            &child,
            &[
                codex_session_meta("child", "parent"),
                codex_turn_context("gpt-5.6-sol"),
                codex_token_count(1_000, 900, 100),
                codex_token_count(1_200, 1_000, 120),
                serde_json::json!({
                    "timestamp": "2026-07-10T03:00:03Z",
                    "type": "event_msg",
                    "payload": { "type": "thread_settings_applied" }
                }),
                codex_token_count(1_300, 1_050, 150),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.source, "codex_cli");
        assert_eq!(
            result.events.len(),
            1,
            "only the post-boundary event is emitted"
        );
        // 2 replay snapshots counted as skipped.
        assert_eq!(result.lines_skipped, 2);
        let ev = &result.events[0];
        assert_eq!(ev.uuid, "codex:thread-v1:child:3");
        assert_eq!(ev.model, "gpt-5.6-sol");
        // fresh input = cache-inclusive delta (100) − cache_read (50).
        assert_eq!(ev.tokens.input, 50);
        assert_eq!(ev.tokens.cache_read, 50);
        assert_eq!(ev.tokens.output, 30);
        assert_eq!(ev.tokens.cache_creation, 0);
    }

    #[test]
    fn codex_subagents_under_same_parent_get_distinct_ids() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let a = sessions.join("a.jsonl");
        let b = sessions.join("b.jsonl");
        write_jsonl(
            &a,
            &[
                codex_session_meta("child-a", "parent"),
                codex_turn_context("gpt-5.6-sol"),
                codex_token_count(100, 50, 10),
            ],
        );
        write_jsonl(
            &b,
            &[
                codex_session_meta("child-b", "parent"),
                codex_turn_context("gpt-5.6-sol"),
                codex_token_count(200, 100, 20),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        let mut uuids: Vec<String> = result.events.iter().map(|e| e.uuid.clone()).collect();
        uuids.sort();
        assert_eq!(
            uuids,
            vec![
                "codex:thread-v1:child-a:1".to_string(),
                "codex:thread-v1:child-b:1".to_string()
            ]
        );
        // fresh inputs: 100−50=50, 200−100=100.
        let by_thread: std::collections::HashMap<&str, u32> = result
            .events
            .iter()
            .map(|e| (e.uuid.rsplit(':').nth(1).unwrap(), e.tokens.input))
            .collect();
        assert_eq!(by_thread["child-a"], 50);
        assert_eq!(by_thread["child-b"], 100);
    }

    #[test]
    fn codex_incremental_emits_only_appended_events() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta("t", "t"),
                codex_turn_context("gpt-5.6-sol"),
                codex_token_count(100, 50, 10),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.events.len(), 1);
        let progress: ScanProgress = delta;
        // Append a second token event — content change bumps mtime past the gate.
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(f, "{}", codex_token_count(300, 100, 40)).unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        // Only the appended event is emitted (fresh input 200−50=150).
        assert_eq!(r2.events.len(), 1);
        assert!(r2.events[0].uuid.ends_with(":2"));
        assert_eq!(r2.events[0].tokens.input, 150);
        assert_eq!(r2.events[0].tokens.cache_read, 50);
        assert_eq!(r2.events[0].tokens.output, 30);
    }

    #[test]
    fn codex_incremental_truncation_self_heals() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta("t", "t"),
                codex_turn_context("gpt-5.6-sol"),
                codex_token_count(100, 50, 10),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.events.len(), 1);
        let progress: ScanProgress = delta;
        // Truncate to fewer lines than the cursor (3) and rewrite with a fresh
        // token event. Without self-heal the stale cursor would make every line
        // "already synced" and the new event would be silently dropped.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_jsonl(
            &file,
            &[codex_session_meta("t", "t"), codex_token_count(200, 0, 20)],
        );
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.events.len(), 1);
        assert_eq!(r2.events[0].tokens.input, 200);
        assert_eq!(r2.events[0].tokens.output, 20);
    }

    // ---- model context appearing after token events (the "unknown" bug) ----

    /// Codex writes `turn_context` only when the model is resolved; early
    /// `token_count` events often carry no `info.model`. Events emitted before
    /// the model line must NOT be permanently stamped "unknown" — they are held
    /// back and flushed with the model once it becomes known.
    #[test]
    fn codex_events_before_model_context_get_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("t1", "/tmp"), // no model in the meta
                codex_token_count(100, 50, 10),
                codex_token_count(300, 100, 40),
                codex_turn_context("gpt-5.6-sol"),
                codex_token_count(500, 200, 60),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 3);
        assert!(
            result.events.iter().all(|e| e.model == "gpt-5.6-sol"),
            "events before the model line must get the model, not 'unknown': {:?}",
            result
                .events
                .iter()
                .map(|e| e.model.clone())
                .collect::<Vec<_>>()
        );
    }

    /// The unknown-model self-heal protocol, PARSER HALF: pass 1 scans the
    /// pre-model prefix and emits the rows as `events` with model "unknown"
    /// (the pass that wrote the store rows); pass 2 — the pass that first sees
    /// the model — must NOT re-emit those stale rows as ordinary events; it
    /// re-offers them in the explicit `corrections` channel (same uuids, model
    /// now known) so the ingest layer's guarded upsert can rewrite exactly the
    /// store rows that still read "unknown". New rows past the cursor stay in
    /// `events`. Every LATER pass re-offers the corrections again (the parser
    /// cannot tell which pre-model rows an earlier pass wrote before the fix —
    /// pre-fix legacy rows heal on any later pass); the store guard turns the
    /// re-offer into a no-op for rows that already carry the model.
    #[test]
    fn codex_incremental_corrects_stale_unknown_events_when_model_appears() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("t1", "/tmp"),
                codex_token_count(100, 50, 10),
                codex_token_count(300, 100, 40),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        // Pass 1 never saw a model → events flushed with the fallback.
        assert_eq!(r1.events.len(), 2);
        assert!(r1.events.iter().all(|e| e.model == "unknown"));
        assert!(
            r1.corrections.is_empty(),
            "no corrections on the first pass"
        );
        let progress: ScanProgress = delta;

        // Model context + one more usage event arrive (session kept running).
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(f, "{}", codex_turn_context("gpt-5.6-sol")).unwrap();
            writeln!(f, "{}", codex_token_count(500, 200, 60)).unwrap();
        }
        let (r2, progress2) = p.collect_incremental(&progress).unwrap();
        // The firing pass re-offers the 2 stale rows as CORRECTIONS (their own
        // channel, not events) and emits the 1 new event — all with the model.
        assert_eq!(
            r2.corrections.len(),
            2,
            "stale rows re-offered as corrections"
        );
        assert_eq!(r2.events.len(), 1, "only the appended event is new");
        assert!(r2.corrections.iter().all(|e| e.model == "gpt-5.6-sol"));
        assert!(r2.events.iter().all(|e| e.model == "gpt-5.6-sol"));
        // The corrections keep the ORIGINAL uuids so the store upsert hits the
        // already-written rows.
        let mut uuids: Vec<String> = r2.corrections.iter().map(|e| e.uuid.clone()).collect();
        uuids.sort();
        assert_eq!(
            uuids,
            vec![
                "codex:thread-v1:t1:1".to_string(),
                "codex:thread-v1:t1:2".to_string()
            ]
        );
        assert!(r2.events[0].uuid.ends_with(":3"));

        // A later pass (cursor past the model line) re-offers the pre-model
        // rows AGAIN as corrections — the parser cannot tell which pre-model
        // rows were written before the fix, so every pass re-offers them with
        // their original uuids and the store's guarded upsert turns the
        // re-offer into a no-op for rows that already carry the model (this is
        // also how pre-fix legacy rows, whose cursor already passed the model
        // line, heal). Plus the one appended event.
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(f, "{}", codex_token_count(700, 300, 80)).unwrap();
        }
        let (r3, _) = p.collect_incremental(&progress2).unwrap();
        assert_eq!(r3.corrections.len(), 2, "corrections re-offered every pass");
        let mut uuids3: Vec<String> = r3.corrections.iter().map(|e| e.uuid.clone()).collect();
        uuids3.sort();
        assert_eq!(
            uuids3,
            vec![
                "codex:thread-v1:t1:1".to_string(),
                "codex:thread-v1:t1:2".to_string()
            ]
        );
        assert_eq!(r3.events.len(), 1, "only the appended event is new");
        assert!(r3.events[0].uuid.ends_with(":4"));
        assert!(r3.corrections.iter().all(|e| e.model == "gpt-5.6-sol"));
    }

    /// Composition seam (parser → ingest → store): the two protocol halves
    /// meet in `ingest_collected`, which must route the parser's `corrections`
    /// through the guarded upsert. Pass 1 writes the pre-model rows "unknown";
    /// pass 2's corrections rewrite exactly those rows — the store rows heal
    /// end-to-end. (The halves' own contracts are unit-tested separately: the
    /// parser half above, the store half in `db::store_ingest`; the legacy
    /// pre-fix-row variant is covered by their composition — re-offered
    /// corrections + guard rewrites only "unknown" rows.)
    #[test]
    fn codex_unknown_model_rows_self_heal_across_collect_passes() {
        use crate::collect::ingest::ingest_collected;
        use crate::config::Paths;
        use crate::db::Store;
        use crate::pricing::seed_book;

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("t1", "/tmp"),
                codex_token_count(100, 50, 10),
                codex_token_count(300, 100, 40),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let paths = Paths::resolve(dir.path());
        let book = seed_book();
        let dev = "0123456789ab";
        let day = "2026-07-10";

        // Collect 1: model context not yet written → rows land as "unknown".
        let (r1, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        let report1 = ingest_collected(&store, &paths, dev, &book, r1).unwrap();
        assert_eq!(report1.rows_inserted, 2);
        let rows = store.usage_for_day_device(day, dev).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.model == "unknown"));

        // The session keeps running: model context + more usage appended.
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(f, "{}", codex_turn_context("gpt-5.6-sol")).unwrap();
            writeln!(f, "{}", codex_token_count(500, 200, 60)).unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        // The routing contract: corrections carried in their own channel, and
        // the ingest counts both the 2 rewritten corrections and the 1 new row.
        assert_eq!(r2.corrections.len(), 2);
        assert_eq!(r2.events.len(), 1);
        let report2 = ingest_collected(&store, &paths, dev, &book, r2).unwrap();
        assert_eq!(report2.rows_inserted, 3, "2 corrections + 1 new row");

        let rows = store.usage_for_day_device(day, dev).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(
            rows.iter().all(|r| r.model == "gpt-5.6-sol"),
            "stale 'unknown' rows must self-heal: {:?}",
            rows.iter().map(|r| r.model.clone()).collect::<Vec<_>>()
        );
    }

    // ---- session + transcript extraction (Codex, this phase) ----

    /// `session_meta` with cwd + a first user message + token usage yields one
    /// RawSession (id/cwd/title/timestamps) and stamps session_id on RawUsage.
    fn codex_session_meta_cwd(thread_id: &str, cwd: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": "2026-07-10T03:00:00Z",
            "type": "session_meta",
            "payload": { "id": thread_id, "session_id": thread_id, "cwd": cwd, "source": "cli" }
        })
    }

    fn codex_response_message(
        id: &str,
        role: &str,
        ts: &str,
        content: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "timestamp": ts,
            "type": "response_item",
            "payload": { "type": "message", "role": role, "id": id, "content": content }
        })
    }

    fn codex_function_call(id: &str, ts: &str, name: &str, arguments: &str) -> serde_json::Value {
        serde_json::json!({
            "timestamp": ts,
            "type": "response_item",
            "payload": { "type": "function_call", "id": id, "name": name, "arguments": arguments }
        })
    }

    #[test]
    fn codex_emits_raw_session_and_stamps_usage_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("sess-xyz.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("sess-xyz", "/home/me/proj"),
                codex_turn_context("gpt-5.6-sol"),
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!("Build a thing"),
                ),
                codex_token_count(100, 50, 10),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();

        // One session, system data from the full file.
        assert_eq!(result.sessions.len(), 1);
        let s = &result.sessions[0];
        assert_eq!(s.id, "sess-xyz");
        assert_eq!(s.source, "codex_cli");
        assert_eq!(s.project_dir, "/home/me/proj");
        assert_eq!(s.title_orig, "Build a thing"); // first user message
        assert_eq!(s.started_at, "2026-07-10T03:00:00Z"); // first line ts
        assert_eq!(s.last_active_at, "2026-07-10T03:00:02Z"); // last line ts

        // Usage carries the session_id (was empty before this phase).
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].session_id, "sess-xyz");
    }

    #[test]
    fn codex_session_title_falls_back_to_cwd_basename() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        // Assistant-only content (no user message) → title = cwd basename.
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("s1", "/home/me/O_cc one"),
                codex_response_message(
                    "m_a1",
                    "assistant",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!([{"type":"output_text","text":"Sure"}]),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "O_cc one");
        assert_eq!(result.sessions[0].project_dir, "/home/me/O_cc one");
    }

    /// Title skips `# AGENTS.md` preamble and `<environment_context>` injection,
    /// landing on the first real user message (mirrors CC-Switch).
    #[test]
    fn codex_session_title_skips_injection_noise() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("s1", "/tmp/project"),
                codex_response_message(
                    "m_u_agents",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!(
                        "# AGENTS.md instructions for /tmp/project\n<INSTRUCTIONS>Do stuff</INSTRUCTIONS>"
                    ),
                ),
                codex_response_message(
                    "m_u_env",
                    "user",
                    "2026-07-10T03:00:11Z",
                    serde_json::json!("<environment_context>\n  <cwd>/tmp/project</cwd>\n</environment_context>"),
                ),
                codex_response_message(
                    "m_u_real",
                    "user",
                    "2026-07-10T03:00:12Z",
                    serde_json::json!("Fix the login bug"),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "Fix the login bug");
    }

    /// VS Code IDE-context wrapper: the real prompt is the body of the last
    /// `## My request for Codex:` section.
    #[test]
    fn codex_session_title_extracts_vscode_ide_request() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let ide = "# Context from my IDE setup:\n\n## Active file: src/main.ts\n\n## My request for Codex:\nFix the session title preview";
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("s1", "/tmp/project"),
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!(ide),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.sessions[0].title_orig,
            "Fix the session title preview"
        );
    }

    /// IDE-context wrapper with NO request section is skipped, and the title
    /// falls through to the next real user message.
    #[test]
    fn codex_session_title_skips_vscode_context_without_request() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let ide = "# Context from my IDE setup:\n\n## Active file: src/main.ts";
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("s1", "/tmp/project"),
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!(ide),
                ),
                codex_response_message(
                    "m_u2",
                    "user",
                    "2026-07-10T03:00:11Z",
                    serde_json::json!("Fix the login bug"),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "Fix the login bug");
    }

    /// Transcript: user/assistant text kept; function_call → a Tool line with
    /// the tool name; function_call_output dropped (lean transcript, like
    /// claude.rs dropping user-role tool_results).
    #[test]
    fn codex_transcript_keeps_text_and_tool_call_drops_output() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("s1", "/tmp"),
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!("list files"),
                ),
                codex_function_call(
                    "call_1",
                    "2026-07-10T03:00:11Z",
                    "shell",
                    r#"{"cmd":["ls"]}"#,
                ),
                serde_json::json!({
                    "timestamp": "2026-07-10T03:00:12Z",
                    "type": "response_item",
                    "payload": { "type": "function_call_output", "id": "out_1", "call_id": "call_1", "output": "file1.txt\nfile2.txt" }
                }),
                codex_response_message(
                    "m_a1",
                    "assistant",
                    "2026-07-10T03:00:13Z",
                    serde_json::json!([{"type":"output_text","text":"Done."}]),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();

        let roles: Vec<_> = result.messages.iter().map(|m| m.role).collect();
        use crate::model::SessionMessageRole::*;
        assert_eq!(roles, vec![User, Tool, Assistant], "output dropped");

        let user = result.messages.iter().find(|m| m.role == User).unwrap();
        assert_eq!(user.content, "list files");
        assert_eq!(user.uuid, "m_u1");

        let tool = result.messages.iter().find(|m| m.role == Tool).unwrap();
        assert_eq!(tool.name.as_deref(), Some("shell"));
        assert!(tool.content.contains("ls"));
        assert_eq!(tool.uuid, "call_1");

        let asst = result
            .messages
            .iter()
            .find(|m| m.role == Assistant)
            .unwrap();
        assert_eq!(asst.content, "Done.");
        assert_eq!(asst.uuid, "m_a1");

        // Every message carries the session_id.
        assert!(result.messages.iter().all(|m| m.session_id == "s1"));
    }

    /// Message uuid is stable (idempotent): a re-collect that only appends one
    /// line emits just that line, and existing lines are not re-emitted.
    #[test]
    fn codex_messages_incremental_past_cursor_with_stable_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("s.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta_cwd("s1", "/tmp"),
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!("first"),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.messages.len(), 1);
        assert_eq!(r1.messages[0].uuid, "m_u1");
        assert_eq!(r1.sessions[0].started_at, "2026-07-10T03:00:00Z");

        // Append a second user line — mtime bump past the gate.
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(
                f,
                "{}",
                codex_response_message(
                    "m_u2",
                    "user",
                    "2026-07-10T03:05:00Z",
                    serde_json::json!("second")
                )
            )
            .unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        // Only the appended line is a new message.
        assert_eq!(r2.messages.len(), 1);
        assert_eq!(r2.messages[0].uuid, "m_u2");
        assert_eq!(r2.messages[0].content, "second");
        // Meta still covers the full file: started_at from line 1, last_active
        // from the appended line.
        assert_eq!(r2.sessions[0].started_at, "2026-07-10T03:00:00Z");
        assert_eq!(r2.sessions[0].last_active_at, "2026-07-10T03:05:00Z");
    }

    /// Sub-agent sessions emit usage (their own post-boundary delta, empty
    /// session_id) but NO RawSession and NO transcript messages.
    #[test]
    fn codex_subagent_session_emits_no_session_and_no_messages() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sessions").join("child.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        write_jsonl(
            &file,
            &[
                codex_session_meta("child", "parent"), // source.subagent ⇒ sub-agent
                codex_turn_context("gpt-5.6-sol"),
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-07-10T03:00:10Z",
                    serde_json::json!("Inspect the project"),
                ),
                codex_token_count(100, 50, 10),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert!(result.sessions.is_empty(), "sub-agent emits no session");
        assert!(result.messages.is_empty(), "sub-agent emits no transcript");
        // Usage still emitted, with empty session_id (no top-level session).
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].session_id, "");
    }

    /// Without `session_meta.id`, the session id falls back to the UUID embedded
    /// in the rollout filename (`rollout-<ts>-<uuid>.jsonl`).
    #[test]
    fn codex_session_id_falls_back_to_filename_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir
            .path()
            .join("sessions")
            .join("rollout-2026-03-06T21-50-12-019cc369-bd7c-7891-b371-7b20b4fe0b18.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        // session_meta with no `id` — only cwd.
        let meta = serde_json::json!({
            "timestamp": "2026-03-06T21:50:12Z",
            "type": "session_meta",
            "payload": { "cwd": "/tmp/project", "source": "cli" }
        });
        write_jsonl(
            &file,
            &[
                meta,
                codex_response_message(
                    "m_u1",
                    "user",
                    "2026-03-06T21:50:13Z",
                    serde_json::json!("hello"),
                ),
            ],
        );
        let p = CodexSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(
            result.sessions[0].id,
            "019cc369-bd7c-7891-b371-7b20b4fe0b18"
        );
    }

    // ---- pure helper unit tests ----

    #[test]
    fn codex_is_uuid_format_validates_8_4_4_4_12() {
        assert!(is_uuid_format("019cc369-bd7c-7891-b371-7b20b4fe0b18"));
        assert!(!is_uuid_format("not-a-uuid"));
        assert!(!is_uuid_format("019cc369-bd7c-7891-b371-7b20b4fe0")); // too short
        assert!(!is_uuid_format("019cc369xbd7cx7891xb371x7b20b4fe0b18")); // dashes wrong
        assert!(!is_uuid_format("019cc369-bd7c-7891-b371-7b20b4fe0b1z")); // non-hex
    }

    #[test]
    fn codex_infer_session_id_from_filename_extracts_trailing_uuid() {
        let p = Path::new("rollout-2026-03-06T21-50-12-019cc369-bd7c-7891-b371-7b20b4fe0b18.jsonl");
        assert_eq!(
            infer_session_id_from_filename(p).as_deref(),
            Some("019cc369-bd7c-7891-b371-7b20b4fe0b18")
        );
        // No UUID tail ⇒ None.
        assert!(infer_session_id_from_filename(Path::new("no-uuid-here.jsonl")).is_none());
    }

    #[test]
    fn codex_title_candidate_filters_noise_and_extracts_ide_request() {
        assert_eq!(
            title_candidate_from_user_message("  How do I deploy?  ").as_deref(),
            Some("How do I deploy?")
        );
        assert!(title_candidate_from_user_message("# AGENTS.md stuff").is_none());
        assert!(title_candidate_from_user_message("<environment_context>").is_none());
        assert!(title_candidate_from_user_message("   ").is_none());
        // IDE wrapper with no request ⇒ None (fall through to next message).
        let ide_no_req = "# Context from my IDE setup:\n\n## Active file: x.ts";
        assert!(title_candidate_from_user_message(ide_no_req).is_none());
        // Inline heading form.
        let ide_inline = "# Context from my IDE setup:\n\n## My request for Codex: Fix the TOC";
        assert_eq!(
            title_candidate_from_user_message(ide_inline).as_deref(),
            Some("Fix the TOC")
        );
        // Block heading form (prompt on following lines); last heading wins.
        let ide_block =
            "# Context from my IDE setup:\n\n## My request for Codex:\nUse the real request";
        assert_eq!(
            title_candidate_from_user_message(ide_block).as_deref(),
            Some("Use the real request")
        );
    }

    #[test]
    fn codex_extract_message_text_handles_string_and_array() {
        assert_eq!(
            extract_codex_message_text(&serde_json::json!("plain")),
            "plain"
        );
        let arr = serde_json::json!([
            { "type": "output_text", "text": "Hello" },
            { "type": "input_text", "text": "World" }
        ]);
        assert_eq!(extract_codex_message_text(&arr), "Hello\nWorld");
        // Unknown item shapes contribute nothing.
        let mixed = serde_json::json!([{ "type": "tool_use", "name": "x" }, { "text": "kept" }]);
        assert_eq!(extract_codex_message_text(&mixed), "kept");
        assert_eq!(extract_codex_message_text(&serde_json::Value::Null), "");
    }
}
