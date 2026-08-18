//! Grok CLI ("Grok Build") session-log parser.
//!
//! One Grok session = one directory under `~/.grok/{sessions,archived_sessions}/
//! <enc-cwd>/<session-id>/`, holding up to three sibling files:
//!   - `summary.json` — meta (`info.id`, `info.cwd`, `generated_title`,
//!     `session_summary`, `created_at`, `last_active_at` / `updated_at`);
//!   - `chat_history.jsonl` — transcript (one JSON line per message, `type` ∈
//!     `user` / `assistant` / `tool` / `system`; `reasoning` is encrypted
//!     internal state and skipped);
//!   - `updates.jsonl` — per-turn usage (JSON-RPC `_x.ai/session/update`
//!     notifications; only `turn_completed` carries billable usage).
//!
//! The directory name is the session id (UUID). Discovery collects every
//! present sibling file; the shared incremental driver gives each its own
//! mtime/line cursor, and [`parse_grok_file`] dispatches by filename:
//!   - `summary.json` → one [`RawSession`] (whole-file re-read, title chain
//!     `generated_title` → `session_summary` → first user message peeked from
//!     `chat_history.jsonl`);
//!   - `chat_history.jsonl` → [`SessionMessage`]s past the line cursor; emits a
//!     degraded `RawSession` (id from the dir, rest empty) only when
//!     `summary.json` is absent, so a session stays visible without its meta;
//!   - `updates.jsonl` → per-model [`RawUsage`]es (each turn an independent
//!     total, never diffed), each stamped with the session id (directory name).
//!
//! A missing sibling degrades gracefully: any one of the three may be absent
//! and the remaining files still produce their stream; no parse panics.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::model::{RawSession, SessionMessage, SessionMessageRole, TokenCounts};

use super::{
    collect_jsonl_incremental, discover_files, normalize_cache_inclusive, truncate, CollectResult,
    DirectoryShape, FileParseOutcome, GateMode, RawUsage, ScanProgress, ScanProgressDelta,
    SourceParser, TITLE_MAX, TRIM_LIMIT,
};

/// Stable source tag — becomes `RawUsage.source` / `RawSession.source` and the
/// DB source column; the single literal behind `name()`, usage, and session
/// construction.
const SOURCE_TAG: &str = "grok_cli";

/// Grok CLI ("Grok Build") session-log parser.
///
/// Reads `~/.grok/{sessions,archived_sessions}/<enc-cwd>/<session-id>/{summary,
/// chat_history, updates}`. See the module docs for the per-file breakdown.
///
/// `updates.jsonl` usage: `inputTokens` is cache-inclusive (it contains
/// `cachedReadTokens`), so it is normalized to fresh at parse; `outputTokens`
/// already includes reasoning (do not add `reasoningTokens`); `cache_creation`
/// is always 0 (Grok exposes no write bucket). One turn may span multiple models
/// (`usage.modelUsage`), each emitted as its own record. The CLI's
/// `costUsdTicks` / `apiDurationMs` are ignored — cost is recomputed from local
/// pricing at ingest.
pub struct GrokSourceParser {
    grok_dir: PathBuf,
}

impl GrokSourceParser {
    /// Root-injection seam: parser rooted at `home/.grok`. The collect
    /// orchestration factory (`all_source_parsers_at`) builds every parser
    /// through this seam, so tests can point the whole chain at a tempdir
    /// fixture instead of the real `~`.
    pub(crate) fn new_at(home: &Path) -> Self {
        Self {
            grok_dir: home.join(".grok"),
        }
    }

    /// Test/override constructor with an explicit Grok dir.
    #[cfg(test)]
    pub(crate) fn with_dir(dir: PathBuf) -> Self {
        Self { grok_dir: dir }
    }
}

impl SourceParser for GrokSourceParser {
    fn name(&self) -> &'static str {
        SOURCE_TAG
    }

    fn discover(&self) -> AppResult<Vec<PathBuf>> {
        // Recursively collect every session sibling file (`summary.json`,
        // `chat_history.jsonl`, `updates.jsonl`) under `sessions/` and
        // `archived_sessions/`. Layout depth varies
        // (`<enc-cwd>/<session-id>/…`), so discovery is by filename,
        // mirroring Grok's session browser; the depth cap guards against
        // pathological nesting. Each file becomes its own cursor entry (keyed
        // by full path), so the three siblings of one session are
        // mtime/line-gated independently. A missing grok dir is not an error.
        Ok(discover_files(
            &[
                DirectoryShape {
                    root: self.grok_dir.join("sessions"),
                    max_depth: Some(9),
                },
                DirectoryShape {
                    root: self.grok_dir.join("archived_sessions"),
                    max_depth: Some(9),
                },
            ],
            is_grok_sibling_file,
        ))
    }

    fn parse(&self, files: &[PathBuf]) -> AppResult<CollectResult> {
        // Delegates to the same `parse_grok_file` dispatcher as
        // `collect_incremental` — each sibling file routes to its own parser by
        // filename — so the test path exercises production logic.
        super::parse_jsonl_full(self, files, parse_grok_file)
    }

    /// Grok's session id is the immediate PARENT DIRECTORY name, not the file
    /// stem — the stem default would mis-delete real sessions on reconcile.
    fn session_ids_seen(&self, files: &[std::path::PathBuf]) -> Vec<String> {
        files
            .iter()
            .map(|p| session_id_of(p))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn collect_incremental(
        &self,
        progress: &ScanProgress,
    ) -> AppResult<(CollectResult, ScanProgressDelta)> {
        collect_jsonl_incremental(self, progress, parse_grok_file)
    }

    /// summary.json is one JSON object re-read in full on each gate pass
    /// (mtime-only — no line cursor); chat_history/updates are append JSONL
    /// (line cursor). Declared per file so the shared driver never pretends a
    /// cursor for the summary.
    fn gate_mode(&self, file: &Path) -> GateMode {
        if file.file_name().and_then(|n| n.to_str()) == Some("summary.json") {
            GateMode::MtimeOnly
        } else {
            GateMode::LineCursor
        }
    }
}

/// Grok session sibling filenames — the whitelist that makes discovery
/// layout-agnostic.
fn is_grok_sibling_file(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|n| n.to_str()),
        Some("summary.json" | "chat_history.jsonl" | "updates.jsonl")
    )
}

/// One-pass dispatcher: route a discovered file to its parser by filename. The
/// shared incremental driver hands us the file text and the 1-based start line
/// (already self-healed on truncation); each parser decides how to use them.
/// `parse` (the full-scan test path) calls through here too, so test and
/// production run identical logic (architecture review #10).
fn parse_grok_file(file: &Path, text: &str, start_line: i64) -> FileParseOutcome {
    match file.file_name().and_then(|n| n.to_str()) {
        Some("updates.jsonl") => parse_grok_updates(file, text, start_line),
        Some("chat_history.jsonl") => parse_grok_chat_history(file, text, start_line),
        Some("summary.json") => parse_grok_summary(file, text),
        _ => FileParseOutcome {
            events: Vec::new(),
            corrections: Vec::new(),
            turn_durations: Vec::new(),
            sessions: Vec::new(),
            messages: Vec::new(),
            skipped: 0,
        },
    }
}

/// The session-id scoping dimension: the immediate parent directory name of a
/// sibling file. Falls back to `"unknown"` only when the path is malformed.
fn session_id_of(file: &Path) -> String {
    file.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

// ---- updates.jsonl: per-turn usage (existing, now session-id-stamped) ----

/// Parse one Grok `updates.jsonl` file into per-call events, skipping lines at
/// or before `start_line` (the incremental cursor). `session_id` is the session
/// directory name — the stable scoping dimension for the per-turn dedup key,
/// and now stamped onto every emitted `RawUsage` (was an empty string).
fn parse_grok_updates(file: &Path, text: &str, start_line: i64) -> FileParseOutcome {
    let session_id = session_id_of(file);
    let mut events = Vec::new();
    let mut skipped = 0u32;
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx as i64 + 1; // 1-based
        if line_no <= start_line {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            skipped += 1; // malformed JSON — a genuine parse failure
            continue;
        };
        // Non-qualifying lines (other notifications / mid-turn snapshots) are
        // normal noise, silently filtered — not counted as skipped.
        if let Some(ev) = parse_grok_notification(&record, &session_id, line_no) {
            events.extend(ev);
        }
    }
    FileParseOutcome {
        events,
        corrections: Vec::new(),
        turn_durations: Vec::new(),
        sessions: Vec::new(),
        messages: Vec::new(),
        skipped,
    }
}

/// Parse one JSON-RPC notification into per-model raw usages, or `None` if the
/// line is not a `turn_completed` usage notification (filtered as noise, not an
/// error). `inputTokens` is cache-inclusive and normalized to fresh here.
fn parse_grok_notification(
    record: &serde_json::Value,
    session_id: &str,
    line_no: i64,
) -> Option<Vec<RawUsage>> {
    if record.get("method").and_then(|v| v.as_str()) != Some("_x.ai/session/update") {
        return None;
    }
    let update = record.get("params").and_then(|p| p.get("update"))?;
    // Only turn_completed carries billable usage; mid-turn snapshots
    // (usage_snapshot) are dropped to avoid double-counting a partial turn.
    // Absent sessionUpdate is passed through for backward compatibility.
    let kind = update.get("sessionUpdate").and_then(|v| v.as_str());
    if kind.is_some() && kind != Some("turn_completed") {
        return None;
    }
    let usage = update.get("usage").filter(|u| u.is_object())?;
    let timestamp = parse_grok_timestamp(record.get("timestamp"))?;

    let prompt_id = update
        .get("prompt_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // prompt_id is the per-turn UUIDv7 (globally unique) — anchoring the dedup
    // key to it (not the file line) survives updates.jsonl rewrites: a rewind
    // truncation shifts surviving events' line numbers, but their prompt_id
    // keys still collide with the store's `(uuid, device_id)` primary key
    // instead of double-counting.
    let turn_key = if prompt_id.is_empty() {
        format!("line{line_no}")
    } else {
        prompt_id.to_string()
    };

    // modelUsage map → one record per model; absent ⇒ top-level counters under
    // an unknown model (pricing layer reconciles the alias). Sorted for
    // deterministic emit order across rescans (object iteration is unspecified).
    let mut per_model: Vec<(String, &serde_json::Value)> = usage
        .get("modelUsage")
        .and_then(|m| m.as_object())
        .map(|map| map.iter().map(|(k, v)| (k.clone(), v)).collect())
        .unwrap_or_default();
    if per_model.is_empty() {
        per_model.push(("unknown".to_string(), usage));
    }
    per_model.sort_by(|a, b| a.0.cmp(&b.0));

    let mut events = Vec::new();
    for (model, counters) in per_model {
        let n = |k: &str| counters.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let input = n("inputTokens");
        let output = n("outputTokens");
        let cached = n("cachedReadTokens");
        // inputTokens is cache-inclusive; normalize to fresh. outputTokens
        // already includes reasoningTokens — do not add them.
        let (fresh_input, clamped_cache_read) = normalize_cache_inclusive(input, cached);
        if fresh_input == 0 && output == 0 && clamped_cache_read == 0 {
            continue; // nothing billable for this model this turn
        }
        events.push(RawUsage {
            uuid: format!("grok:turn:{session_id}:{turn_key}:{model}"),
            timestamp: timestamp.clone(),
            model,
            source: SOURCE_TAG.to_string(),
            session_id: session_id.to_string(),
            tokens: TokenCounts {
                input: fresh_input,
                output,
                cache_creation: 0,
                cache_read: clamped_cache_read,
            },
            ..Default::default()
        });
    }
    Some(events)
}

// ---- summary.json: session meta (one RawSession per directory) ----

/// Parse `summary.json` into one [`RawSession`]. Whole-file re-read each pass
/// (meta is refreshable). Field fallbacks: id = `info.id` (else dir name);
/// project_dir = `info.cwd`; title = `generated_title` → `session_summary` →
/// first user message peeked from the sibling `chat_history.jsonl`; times =
/// `created_at` / (`last_active_at` | `updated_at`), each ISO-or-epoch. Any
/// malformed shape degrades to the directory-name id with empty fields — never
/// panics, never counts as skipped (a half-written summary is re-read next pass).
fn parse_grok_summary(file: &Path, text: &str) -> FileParseOutcome {
    let session = raw_session_from_summary(file, text);
    FileParseOutcome {
        events: Vec::new(),
        corrections: Vec::new(),
        turn_durations: Vec::new(),
        sessions: vec![session],
        messages: Vec::new(),
        skipped: 0,
    }
}

/// Build the [`RawSession`] from `summary.json` text, with the full title chain
/// and graceful degradation. Split out so the degraded title fallback (peek the
/// sibling transcript) reads the file once instead of branching inline.
fn raw_session_from_summary(file: &Path, text: &str) -> RawSession {
    let dir_id = session_id_of(file);
    let mut session = RawSession {
        id: dir_id,
        source: SOURCE_TAG.to_string(),
        project_dir: String::new(),
        title_orig: String::new(),
        started_at: String::new(),
        last_active_at: String::new(),
        agent_type: String::new(),
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        // id: info.id wins when non-empty; otherwise the directory name stays.
        if let Some(info_id) = v
            .get("info")
            .and_then(|i| i.get("id"))
            .and_then(|i| i.as_str())
            .filter(|s| !s.is_empty())
        {
            session.id = info_id.to_string();
        }
        if let Some(cwd) = v
            .get("info")
            .and_then(|i| i.get("cwd"))
            .and_then(|c| c.as_str())
        {
            session.project_dir = cwd.to_string();
        }
        // Title sources from summary.json only; first-user is the final fallback
        // (peeked below) so generated_title > session_summary > first user.
        let generated = v
            .get("generated_title")
            .and_then(|t| t.as_str())
            .map(str::trim)
            .unwrap_or("");
        let summary_text = v
            .get("session_summary")
            .and_then(|t| t.as_str())
            .map(str::trim)
            .unwrap_or("");
        let from_summary = if !generated.is_empty() {
            generated
        } else {
            summary_text
        };
        session.title_orig = truncate(from_summary, TITLE_MAX);
        session.started_at = parse_grok_timestamp(v.get("created_at")).unwrap_or_default();
        session.last_active_at =
            parse_grok_timestamp(v.get("last_active_at").or_else(|| v.get("updated_at")))
                .unwrap_or_default();
    }
    // Final title fallback: peek the sibling chat_history.jsonl for the first
    // real user message. Only when summary.json itself yielded no title.
    if session.title_orig.is_empty() {
        if let Some(parent) = file.parent() {
            if let Some(first_user) = peek_first_user_message(&parent.join("chat_history.jsonl")) {
                session.title_orig = truncate(&first_user, TITLE_MAX);
            }
        }
    }
    session
}

/// Scan `chat_history.jsonl` from disk until the first `user` line with
/// non-empty text content, returning that text (un-truncated; caller caps it).
/// Bounded by the location of the first user turn (typically line 1–3). Returns
/// `None` on any open/parse miss — used only as the title fallback, so a missing
/// or unparseable transcript simply leaves the title empty.
fn peek_first_user_message(chat_path: &Path) -> Option<String> {
    let file = std::fs::File::open(chat_path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        if record.get("type").and_then(|t| t.as_str()) != Some("user") {
            continue;
        }
        if let Some(content) = record.get("content") {
            let text = extract_grok_content(content);
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

// ---- chat_history.jsonl: transcript messages (line-incremental) ----

/// Parse `chat_history.jsonl` into [`SessionMessage`]s past `start_line`. Role
/// mapping: user/assistant/tool/system → the four UI roles; `reasoning` is
/// dropped (encrypted/internal). Each line carries a stable line-based uuid
/// (Grok lines have no id) so appends are idempotent across re-collects. When
/// `summary.json` is ABSENT from the same directory, a degraded [`RawSession`]
/// (id = dir name, all other fields empty) is emitted alongside the messages,
/// so a transcript-only session stays visible in the session list.
fn parse_grok_chat_history(file: &Path, text: &str, start_line: i64) -> FileParseOutcome {
    let session_id = session_id_of(file);
    let summary_present = file
        .parent()
        .map(|d| d.join("summary.json").exists())
        .unwrap_or(false);
    let mut messages = Vec::new();
    let mut skipped = 0u32;
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx as i64 + 1; // 1-based, matching the cursor
        if line_no <= start_line {
            continue;
        }
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<serde_json::Value>(line) else {
            skipped += 1; // malformed JSON line — a genuine parse failure
            continue;
        };
        let kind = record.get("type").and_then(|t| t.as_str()).unwrap_or("");
        // reasoning (and any unknown type) is skipped as noise, not a failure.
        let role = match kind {
            "user" => SessionMessageRole::User,
            "assistant" => SessionMessageRole::Assistant,
            "tool" => SessionMessageRole::Tool,
            "system" => SessionMessageRole::System,
            _ => continue,
        };
        let content = record
            .get("content")
            .map(extract_grok_content)
            .unwrap_or_default();
        let content = truncate(content.trim(), TRIM_LIMIT);
        if content.is_empty() {
            continue;
        }
        let ts = parse_grok_timestamp(record.get("timestamp").or_else(|| record.get("ts")))
            .unwrap_or_default();
        let name = if matches!(role, SessionMessageRole::Tool) {
            record
                .get("name")
                .or_else(|| record.get("tool_name"))
                .and_then(|n| n.as_str())
                .map(str::to_string)
        } else {
            None
        };
        messages.push(SessionMessage {
            uuid: format!("grok:msg:{session_id}:line{line_no}"),
            session_id: session_id.clone(),
            role,
            ts,
            model: None,
            name,
            content,
        });
    }
    // Degraded session: only when summary.json is missing. Keeps the directory
    // visible as a session (id = dir name) with empty meta per the spec; the
    // ingest layer attaches the transcript + usage rows by session_id.
    let sessions = if summary_present {
        Vec::new()
    } else {
        vec![RawSession {
            id: session_id,
            source: SOURCE_TAG.to_string(),
            project_dir: String::new(),
            title_orig: String::new(),
            started_at: String::new(),
            last_active_at: String::new(),
            agent_type: String::new(),
        }]
    };
    FileParseOutcome {
        events: Vec::new(),
        corrections: Vec::new(),
        turn_durations: Vec::new(),
        sessions,
        messages,
        skipped,
    }
}

/// Extract text from a Grok message `content` field: a plain string, or an
/// array of `{type:"text", text:…}` blocks joined with `\n`. Other block shapes
/// (and non-string/array values) yield empty — reasoning/internal blocks never
/// reach here (their lines are filtered upstream by type).
fn extract_grok_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| {
                    if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                        item.get("text")
                            .and_then(|t| t.as_str())
                            .map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect();
            parts.join("\n")
        }
        _ => String::new(),
    }
}

// ---- shared timestamp parse (epoch sec / epoch ms / RFC3339) ----

/// Parse a Grok timestamp value (epoch seconds, epoch milliseconds if large, or
/// an RFC3339 string) into an ISO8601 UTC string. Returns `None` if absent or
/// unparseable. Shared across updates.jsonl / summary.json / chat_history.jsonl
/// so every Grok timestamp format funnels through one normalizer.
fn parse_grok_timestamp(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    if let Some(n) = value.as_i64() {
        // >1e11 ⇒ milliseconds (defensive, mirrors CC-Switch's threshold).
        let secs = if n > 100_000_000_000 { n / 1000 } else { n };
        return Some(crate::time::epoch_to_iso(secs));
    }
    value
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| {
            dt.with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    fn grok_event_line(epoch: i64, prompt_id: &str, model_usage: &str) -> String {
        format!(
            r#"{{"timestamp":{epoch},"method":"_x.ai/session/update","params":{{"update":{{"sessionUpdate":"turn_completed","prompt_id":"{prompt_id}","usage":{{"modelUsage":{{{model_usage}}}}}}}}}}}"#
        )
    }

    /// One model's counters, deliberately carrying `reasoningTokens` (must NOT
    /// be added to output), `apiDurationMs`, and `costUsdTicks` (both ignored).
    fn grok_model(model: &str, input: u64, output: u64, cached: u64) -> String {
        format!(
            r#""{model}":{{"inputTokens":{input},"outputTokens":{output},"cachedReadTokens":{cached},"reasoningTokens":3,"modelCalls":1,"apiDurationMs":1000,"costUsdTicks":999}}"#
        )
    }

    /// Write only `updates.jsonl` into a session dir (legacy test fixture).
    fn write_grok_session(dir: &Path, session_id: &str, lines: &[String]) -> PathBuf {
        let session_dir = dir.join("sessions").join("enc-project").join(session_id);
        std::fs::create_dir_all(&session_dir).unwrap();
        let path = session_dir.join("updates.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        path
    }

    /// Write a full Grok session directory with whichever siblings are given.
    /// `summary` = None ⇒ no summary.json; empty `chat` / `updates` ⇒ skipped.
    fn write_grok_full_session(
        dir: &Path,
        session_id: &str,
        summary: Option<&str>,
        chat: &[&str],
        updates: &[String],
    ) -> PathBuf {
        let session_dir = dir.join("sessions").join("enc-project").join(session_id);
        std::fs::create_dir_all(&session_dir).unwrap();
        if let Some(s) = summary {
            std::fs::write(session_dir.join("summary.json"), s).unwrap();
        }
        if !chat.is_empty() {
            let mut f = fs::File::create(session_dir.join("chat_history.jsonl")).unwrap();
            for l in chat {
                writeln!(f, "{l}").unwrap();
            }
        }
        if !updates.is_empty() {
            let mut f = fs::File::create(session_dir.join("updates.jsonl")).unwrap();
            for l in updates {
                writeln!(f, "{l}").unwrap();
            }
        }
        session_dir
    }

    // ============= existing updates.jsonl-only coverage =============

    #[test]
    fn grok_discover_missing_dir_returns_empty() {
        let base = tempfile::tempdir().unwrap();
        let p = GrokSourceParser::with_dir(base.path().join("nope"));
        assert!(p.discover().unwrap().is_empty());
    }

    #[test]
    fn grok_parses_turn_completed_and_ignores_noise() {
        let dir = tempfile::tempdir().unwrap();
        let lines = vec![
            // Wrong method → filtered.
            r#"{"timestamp":1700000000,"method":"session/update","params":{"update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":1}}}}"#.to_string(),
            // Mid-turn snapshot: has usage but is not turn_completed → dropped
            // (would double-count a partial turn alongside its turn_completed).
            r#"{"timestamp":1700000000,"method":"_x.ai/session/update","params":{"update":{"sessionUpdate":"usage_snapshot","prompt_id":"px","usage":{"inputTokens":9999,"outputTokens":9}}}}"#.to_string(),
            // Malformed JSON → counts as skipped.
            "not json".to_string(),
            grok_event_line(1_700_000_000, "p1", &grok_model("grok-4.5-build", 16632, 104, 0)),
        ];
        write_grok_session(dir.path(), "s1", &lines);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.source, "grok_cli");
        assert_eq!(result.events.len(), 1);
        assert_eq!(
            result.lines_skipped, 1,
            "only the malformed line counts as skipped"
        );
        let ev = &result.events[0];
        assert_eq!(ev.uuid, "grok:turn:s1:p1:grok-4.5-build");
        assert_eq!(ev.model, "grok-4.5-build");
        assert_eq!(ev.tokens.input, 16632);
        assert_eq!(ev.tokens.output, 104, "reasoningTokens (3) NOT added");
        assert_eq!(ev.tokens.cache_read, 0);
        assert_eq!(ev.tokens.cache_creation, 0);
        assert_eq!(ev.timestamp, "2023-11-14T22:13:20.000Z");
    }

    /// Each turn_completed is an independent per-turn total — never diffed.
    /// Diffing would shrink turn 2 to a tiny delta (the bug CC-Switch hit).
    #[test]
    fn grok_records_each_turn_at_face_value_no_diff() {
        let dir = tempfile::tempdir().unwrap();
        let lines = vec![
            grok_event_line(
                1_700_000_000,
                "p1",
                &grok_model("grok-4.5-build", 17294, 28, 11136),
            ),
            grok_event_line(
                1_700_000_060,
                "p2",
                &grok_model("grok-4.5-build", 17347, 56, 17280),
            ),
        ];
        write_grok_session(dir.path(), "s2", &lines);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 2);
        let by_prompt: std::collections::HashMap<&str, &RawUsage> =
            result.events.iter().map(|e| (e.uuid.as_str(), e)).collect();
        // Face value, cache-inclusive input normalized to fresh.
        let p1 = by_prompt["grok:turn:s2:p1:grok-4.5-build"];
        let p2 = by_prompt["grok:turn:s2:p2:grok-4.5-build"];
        assert_eq!(p1.tokens.input, 17294 - 11136);
        assert_eq!(p1.tokens.cache_read, 11136);
        assert_eq!(p2.tokens.input, 17347 - 17280);
        assert_eq!(p2.tokens.cache_read, 17280);
    }

    #[test]
    fn grok_identical_turns_both_counted() {
        let dir = tempfile::tempdir().unwrap();
        let lines = vec![
            grok_event_line(
                1_700_000_000,
                "p1",
                &grok_model("grok-4.5-build", 100, 10, 0),
            ),
            grok_event_line(
                1_700_000_060,
                "p2",
                &grok_model("grok-4.5-build", 100, 10, 0),
            ),
        ];
        write_grok_session(dir.path(), "s3", &lines);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.events.len(),
            2,
            "identical turns are two real usages, not a zero delta"
        );
    }

    #[test]
    fn grok_multi_model_emits_one_row_per_model() {
        let dir = tempfile::tempdir().unwrap();
        let both = format!(
            "{},{}",
            grok_model("grok-4.5-build", 100, 10, 0),
            grok_model("grok-4.3", 30, 3, 10),
        );
        let lines = vec![grok_event_line(1_700_000_000, "p1", &both)];
        write_grok_session(dir.path(), "s4", &lines);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 2);
        // Deterministic order: sorted by model name.
        assert!(result.events[0].uuid.ends_with(":grok-4.3"));
        assert!(result.events[1].uuid.ends_with(":grok-4.5-build"));
        let g43 = &result.events[0];
        assert_eq!(g43.tokens.input, 20, "cache-inclusive 30 minus cached 10");
        assert_eq!(g43.tokens.cache_read, 10);
    }

    #[test]
    fn grok_missing_model_usage_falls_back_to_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let line = r#"{"timestamp":1700000000,"method":"_x.ai/session/update","params":{"update":{"prompt_id":"p1","usage":{"inputTokens":100,"outputTokens":10,"cachedReadTokens":5}}}}"#.to_string();
        write_grok_session(dir.path(), "s5", &[line]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].model, "unknown");
        assert_eq!(result.events[0].tokens.input, 95);
        assert_eq!(result.events[0].tokens.cache_read, 5);
    }

    #[test]
    fn grok_archived_sessions_are_also_discovered() {
        let dir = tempfile::tempdir().unwrap();
        // Same filename under archived_sessions/ must be picked up too.
        let arch = dir
            .path()
            .join("archived_sessions")
            .join("enc")
            .join("arch1");
        std::fs::create_dir_all(&arch).unwrap();
        std::fs::write(
            arch.join("updates.jsonl"),
            grok_event_line(1_700_000_000, "p1", &grok_model("grok-4.5-build", 10, 1, 0)),
        )
        .unwrap();
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].uuid, "grok:turn:arch1:p1:grok-4.5-build");
    }

    #[test]
    fn grok_incremental_emits_only_appended_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_grok_session(
            dir.path(),
            "s6",
            &[grok_event_line(
                1_700_000_000,
                "p1",
                &grok_model("grok-4.5-build", 100, 10, 0),
            )],
        );
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.events.len(), 1);
        let progress: ScanProgress = delta;
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            writeln!(
                f,
                "{}",
                grok_event_line(
                    1_700_000_060,
                    "p2",
                    &grok_model("grok-4.5-build", 250, 30, 0)
                )
            )
            .unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.events.len(), 1);
        assert_eq!(r2.events[0].uuid, "grok:turn:s6:p2:grok-4.5-build");
    }

    // ============= session meta (summary.json) =============

    /// summary.json yields one RawSession: generated_title wins over
    /// session_summary, cwd → project_dir, epoch timestamps → ISO.
    #[test]
    fn grok_session_meta_from_summary_json() {
        let dir = tempfile::tempdir().unwrap();
        let summary = r#"{"info":{"id":"s-meta","cwd":"/work/proj"},"generated_title":"My Title","session_summary":"summary text","created_at":1700000000,"last_active_at":1700000060}"#;
        write_grok_full_session(dir.path(), "s-meta", Some(summary), &[], &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions.len(), 1);
        let s = &result.sessions[0];
        assert_eq!(s.id, "s-meta");
        assert_eq!(s.source, "grok_cli");
        assert_eq!(s.project_dir, "/work/proj");
        assert_eq!(
            s.title_orig, "My Title",
            "generated_title beats session_summary"
        );
        assert_eq!(s.started_at, "2023-11-14T22:13:20.000Z");
        assert_eq!(s.last_active_at, "2023-11-14T22:14:20.000Z");
    }

    /// Without generated_title, session_summary is the title.
    #[test]
    fn grok_session_title_falls_back_to_session_summary() {
        let dir = tempfile::tempdir().unwrap();
        let summary = r#"{"info":{"id":"s-sum","cwd":"/p"},"session_summary":"Summary only"}"#;
        write_grok_full_session(dir.path(), "s-sum", Some(summary), &[], &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "Summary only");
    }

    /// With neither generated_title nor session_summary, the title falls back to
    /// the first user message peeked from the sibling chat_history.jsonl.
    #[test]
    fn grok_session_title_falls_back_to_first_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let summary = r#"{"info":{"id":"s-user","cwd":"/p"}}"#; // no title fields
        let chat = [
            r#"{"type":"assistant","content":"reply"}"#,
            r#"{"type":"user","content":"First prompt here","timestamp":1700000000}"#,
        ];
        write_grok_full_session(dir.path(), "s-user", Some(summary), &chat, &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "First prompt here");
    }

    /// last_active_at falls back to updated_at when last_active_at is absent.
    #[test]
    fn grok_session_last_active_falls_back_to_updated_at() {
        let dir = tempfile::tempdir().unwrap();
        let summary =
            r#"{"info":{"id":"s-up","cwd":"/p"},"created_at":1700000000,"updated_at":1700000120}"#;
        write_grok_full_session(dir.path(), "s-up", Some(summary), &[], &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.sessions[0].last_active_at, "2023-11-14T22:15:20.000Z",
            "updated_at used when last_active_at missing"
        );
    }

    /// Timestamp fields accept epoch-seconds, epoch-millis, and RFC3339 strings.
    #[test]
    fn grok_timestamp_field_accepts_iso_epoch_seconds_and_millis() {
        let dir = tempfile::tempdir().unwrap();
        let enc = dir.path().join("sessions").join("enc");
        let mk = |id: &str, ts: &str| {
            let d = enc.join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join("summary.json"),
                format!(r#"{{"info":{{"id":"{id}","cwd":"/p"}},"created_at":{ts}}}"#),
            )
            .unwrap();
        };
        mk("ts-sec", "1700000000"); // epoch seconds
        mk("ts-ms", "1700000000000"); // epoch milliseconds
        mk("ts-iso", r#""2023-11-14T22:13:20Z""#); // RFC3339 string
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        let by_id: std::collections::HashMap<&str, &RawSession> =
            result.sessions.iter().map(|s| (s.id.as_str(), s)).collect();
        assert_eq!(by_id["ts-sec"].started_at, "2023-11-14T22:13:20.000Z");
        assert_eq!(by_id["ts-ms"].started_at, "2023-11-14T22:13:20.000Z");
        assert_eq!(by_id["ts-iso"].started_at, "2023-11-14T22:13:20.000Z");
    }

    /// Malformed summary.json degrades to a directory-name id with empty meta —
    /// no panic, no skip count (a half-write re-reads next pass).
    #[test]
    fn grok_summary_malformed_degrades_to_dir_id() {
        let dir = tempfile::tempdir().unwrap();
        write_grok_full_session(dir.path(), "s-bad", Some("{not json"), &[], &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions.len(), 1);
        let s = &result.sessions[0];
        assert_eq!(s.id, "s-bad", "directory name is the id fallback");
        assert!(s.project_dir.is_empty());
        assert!(s.title_orig.is_empty());
        assert_eq!(
            result.lines_skipped, 0,
            "a malformed summary is re-read next pass, not counted as skipped"
        );
    }

    // ============= transcript (chat_history.jsonl) =============

    /// chat_history maps user/assistant/tool/system to SessionMessages;
    /// reasoning is filtered; tool name is extracted; `ts` is an accepted alt
    /// timestamp key; array content is joined.
    #[test]
    fn grok_chat_history_messages_and_reasoning_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let summary = r#"{"info":{"id":"s-chat","cwd":"/p"}}"#;
        let chat = [
            r#"{"type":"user","content":"hi","timestamp":1700000000}"#,
            // reasoning carries encrypted/internal state — must be dropped.
            r#"{"type":"reasoning","content":"secret internal"}"#,
            r#"{"type":"assistant","content":[{"type":"text","text":"Sure"}],"timestamp":1700000060}"#,
            // `ts` alt key + epoch + tool name extraction.
            r#"{"type":"tool","name":"Read","content":"file contents","ts":1700000120}"#,
        ];
        write_grok_full_session(dir.path(), "s-chat", Some(summary), &chat, &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();

        let roles: Vec<_> = result.messages.iter().map(|m| m.role).collect();
        use crate::model::SessionMessageRole::*;
        assert_eq!(roles, vec![User, Assistant, Tool], "reasoning filtered");
        assert!(
            !result.messages.iter().any(|m| m.content.contains("secret")),
            "reasoning content never leaks"
        );
        // tool name carried through.
        let tool = result.messages.iter().find(|m| m.role == Tool).unwrap();
        assert_eq!(tool.name.as_deref(), Some("Read"));
        // `ts` alt key + epoch → ISO.
        assert_eq!(tool.ts, "2023-11-14T22:15:20.000Z");
        assert_eq!(tool.session_id, "s-chat");
        // assistant array content joined.
        let asst = result
            .messages
            .iter()
            .find(|m| m.role == Assistant)
            .unwrap();
        assert_eq!(asst.content, "Sure");
        // user content + line-based stable uuid.
        let user = result.messages.iter().find(|m| m.role == User).unwrap();
        assert_eq!(user.content, "hi");
        assert_eq!(user.uuid, "grok:msg:s-chat:line1");
    }

    /// summary.json ABSENT: chat_history still yields its messages AND a degraded
    /// RawSession (id = dir, empty meta) so the session stays visible.
    #[test]
    fn grok_summary_missing_degrades_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let chat = [r#"{"type":"user","content":"Hello degraded","timestamp":1700000000}"#];
        // No summary.json — only chat_history.
        write_grok_full_session(dir.path(), "s-deg", None, &chat, &[]);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.sessions.len(),
            1,
            "session visible even without summary.json"
        );
        let s = &result.sessions[0];
        assert_eq!(s.id, "s-deg", "directory name fallback id");
        assert!(s.project_dir.is_empty(), "project_dir left empty");
        assert!(s.title_orig.is_empty(), "title left empty");
        // messages still extracted.
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].content, "Hello degraded");
        assert_eq!(result.messages[0].session_id, "s-deg");
    }

    // ============= session_id on RawUsage (was empty) =============

    /// updates.jsonl usages now carry the session id (directory name), so they
    /// attach to the session in the ingest layer.
    #[test]
    fn grok_usage_carries_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let updates = vec![grok_event_line(
            1_700_000_000,
            "p1",
            &grok_model("grok-4.5-build", 100, 10, 0),
        )];
        write_grok_full_session(dir.path(), "s-usage", None, &[], &updates);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(
            result.events[0].session_id, "s-usage",
            "RawUsage.session_id filled with the directory name"
        );
    }

    // ============= three-file coordination =============

    /// All three siblings present: exactly one RawSession (from summary.json),
    /// messages from chat_history, usages from updates — no duplicates.
    #[test]
    fn grok_three_siblings_emit_one_session_messages_and_usages() {
        let dir = tempfile::tempdir().unwrap();
        let summary = r#"{"info":{"id":"s-full","cwd":"/proj"},"generated_title":"Full","created_at":1700000000,"last_active_at":1700000120}"#;
        let chat = [
            r#"{"type":"user","content":"hi","timestamp":1700000000}"#,
            r#"{"type":"assistant","content":"bye","timestamp":1700000060}"#,
        ];
        let updates = vec![grok_event_line(
            1_700_000_000,
            "p1",
            &grok_model("grok-4.5-build", 100, 10, 0),
        )];
        write_grok_full_session(dir.path(), "s-full", Some(summary), &chat, &updates);
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.sessions.len(),
            1,
            "exactly one session (no duplicate)"
        );
        assert_eq!(result.sessions[0].id, "s-full");
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].session_id, "s-full");
        assert!(
            result.messages.iter().all(|m| m.session_id == "s-full"),
            "every message carries the session id"
        );
    }

    /// Gate modes are declared per file: summary.json is mtime-only (no line
    /// cursor), chat/updates are line-cursor append logs. The driver must
    /// record a line offset ONLY for the line-cursor files — the fake-cursor
    /// contract this pins is the whole point of the three-state declaration.
    #[test]
    fn grok_gate_modes_declared_per_file() {
        let p = GrokSourceParser::with_dir(tempfile::tempdir().unwrap().path().to_path_buf());
        assert_eq!(
            p.gate_mode(Path::new("/x/summary.json")),
            GateMode::MtimeOnly,
            "summary re-reads whole — no line cursor"
        );
        assert_eq!(
            p.gate_mode(Path::new("/x/chat_history.jsonl")),
            GateMode::LineCursor
        );
        assert_eq!(
            p.gate_mode(Path::new("/x/updates.jsonl")),
            GateMode::LineCursor
        );
    }

    /// chat_history.jsonl incremental: only appended lines yield messages.
    #[test]
    fn grok_incremental_chat_history_emits_only_appended_messages() {
        let dir = tempfile::tempdir().unwrap();
        let summary = r#"{"info":{"id":"s-inc","cwd":"/p"}}"#;
        let chat = [r#"{"type":"user","content":"first","timestamp":1700000000}"#];
        let session_dir = write_grok_full_session(dir.path(), "s-inc", Some(summary), &chat, &[]);
        let chat_path = session_dir.join("chat_history.jsonl");
        let p = GrokSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.messages.len(), 1, "first pass: one message");
        // The honest cursor contract: the summary's cursor is mtime-only
        // (line offset 0), the chat's is a real line cursor.
        let key_summary = crate::source_parser::scan_progress_key(&session_dir.join("summary.json"));
        let key_chat = crate::source_parser::scan_progress_key(&chat_path);
        assert_eq!(
            delta.get(&key_summary).unwrap().last_line_offset,
            0,
            "summary.json has no line cursor (mtime-only)"
        );
        assert!(
            delta.get(&key_chat).unwrap().last_line_offset >= 1,
            "chat_history.jsonl advances a real line cursor"
        );
        let progress: ScanProgress = delta;
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&chat_path)
                .unwrap();
            writeln!(
                f,
                r#"{{"type":"user","content":"second","timestamp":1700000060}}"#
            )
            .unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.messages.len(), 1, "only the appended line is emitted");
        assert_eq!(r2.messages[0].content, "second");
    }
}
