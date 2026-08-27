//! Claude Code session-log parser.

use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::model::{ServerToolUse, SessionMessage, SessionMessageRole, TokenCounts};

use super::{
    collect_jsonl_incremental, discover_files, is_jsonl_file, truncate, CollectResult,
    DirectoryShape, RawTurnDuration, RawUsage, ScanProgress, ScanProgressDelta, SessionExtras,
    SessionMetaAcc, SourceParser, TRIM_LIMIT,
};

/// Stable source tag — becomes `RawUsage.source` / `RawSession.source` and the
/// DB source column; the single literal behind `name()`, usage, and session
/// construction.
const SOURCE_TAG: &str = "claude_code";

/// Claude Code session-log parser.
///
/// Reads `~/.claude/projects/**/*.jsonl`; each line is a JSON event. Assistant
/// events carry `message.usage` (token four-pack + server tool use + service
/// tier + iterations) and `message.stop_reason`. `system` events with
/// `subtype:"turn_duration"` carry `durationMs`. Top-level `timestamp` and
/// `uuid` identify each event.
pub struct ClaudeCodeSourceParser {
    /// Root of the Claude projects dir (overridable for tests).
    projects_dir: PathBuf,
}

impl ClaudeCodeSourceParser {
    /// Root-injection seam: parser rooted at `home/.claude/projects`. The
    /// collect orchestration factory (`all_source_parsers_at`) builds every
    /// parser through this seam, so tests can point the whole chain at a
    /// tempdir fixture instead of the real `~`.
    pub(crate) fn new_at(home: &Path) -> Self {
        Self {
            projects_dir: home.join(".claude").join("projects"),
        }
    }

    /// Test/override constructor with an explicit projects dir.
    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { projects_dir: dir }
    }

    /// Fold one JSONL file's text into a per-file parse outcome. Three streams
    /// are produced:
    ///   - per-call usages + per-turn durations (lines past `start_line`, the
    ///     incremental cursor — same as before, message-id-deduped);
    ///   - one session-meta record (`RawSession`) covering the WHOLE file —
    ///     system data is refreshable, so every pass re-reads first/last ts,
    ///     the first-seen cwd (project_dir), and the title sources
    ///     (custom-title > summary > first user message > project dir
    ///     basename), latest-seen at each title level; accumulated through
    ///     [`super::SessionMetaAcc`] (the shared skeleton — see its contract
    ///     for the full-file-vs-cursor invariant);
    ///   - transcript messages (lines past `start_line` only — incremental, so a
    ///     re-collect appends only new lines to `sessions/<id>.jsonl`).
    ///
    /// `session_id` is the file stem (Claude = one session per jsonl). Dedup is
    /// scoped per file: a message id is unique within one jsonl, so per-file is
    /// correct, and sharing this fold between `parse` and `collect_incremental`
    /// keeps the test and production paths identical (architecture review #10).
    ///
    /// Invariant: the stored `RawUsage.uuid` is the source **event** uuid, NOT
    /// the dedup key (the message id) — re-keying stored rows to the message id
    /// would mass-duplicate on first run. The message id is the map key only.
    fn fold_file(file: &Path, text: &str, start_line: i64) -> super::FileParseOutcome {
        let session_id = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        // Subagent sidechain sessions (agent-*.jsonl) carry a sidecar
        // `<stem>.meta.json` with the agent type + task description; both
        // become session metadata (type tag + title source). A missing sidecar
        // (deleted / non-Claude file) degrades to the generic "agent" tag.
        let is_agent = session_id.starts_with("agent-");
        let agent_meta = if is_agent {
            file.parent()
                .map(|dir| dir.join(format!("{session_id}.meta.json")))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|s| serde_json::from_str::<AgentMeta>(&s).ok())
        } else {
            None
        };
        // Parent link: Claude Code writes subagent sidechain files at
        // `<project>/<parent-session-id>/subagents/agent-*.jsonl` — the
        // directory chain IS the explicit parent field (written by Claude Code
        // itself; on the machine this was derived from, 261/261 agent files
        // live in that shape and 53/53 parent dirs pair with a real parent
        // session file — no heuristic inference). Old Claude Code versions
        // wrote agent files flat in the project dir: no link exists there, so
        // those rows keep `""` and display top-level (`agent_type` still set).
        let parent_session_id = if is_agent {
            file.parent()
                .filter(|dir| dir.file_name().and_then(|n| n.to_str()) == Some("subagents"))
                .and_then(|dir| dir.parent())
                .and_then(|grand| grand.file_name().and_then(|n| n.to_str()))
                .map(str::to_string)
                .unwrap_or_default()
        } else {
            String::new()
        };
        let mut events_by_mid: std::collections::HashMap<String, RawUsage> =
            std::collections::HashMap::new();
        let mut turn_durations = Vec::new();
        let mut messages: Vec<SessionMessage> = Vec::new();
        let mut skipped = 0u32;

        // Session meta accumulates through the shared [`super::SessionMetaAcc`]
        // skeleton — tracked over the FULL file, not just the cursor tail (system
        // data is refreshable, and started_at/cwd/title would be lost if we only
        // saw the appended lines on a re-collect); every parseable line feeds it.
        let mut meta = SessionMetaAcc::default();
        // Claude-specific title sources ABOVE the shared first-user layer, both
        // LATEST non-empty wins (unlike the user title these must refresh):
        //   - summary — Claude may emit it later in the file (e.g. after a
        //     /compact); the title follows that update instead of freezing on
        //     the first-seen value;
        //   - custom_title — the user's manual session name; a mid-session
        //     rename must refresh the title (the first-wins bug CC-Switch has).
        // Both land in `finish` as extra levels ahead of the user title.
        let mut summary = String::new();
        let mut custom_title: Option<String> = None;

        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx as i64 + 1; // 1-based, matching the cursor
            let line = raw.trim();
            if line.is_empty() {
                continue;
            }
            let ev = match serde_json::from_str::<SessionEvent>(line) {
                Ok(ev) => ev,
                Err(_) => {
                    // Only count a malformed line as skipped when it is past the
                    // cursor (the incremental tail); already-counted lines are
                    // not re-counted on a re-collect.
                    if line_no > start_line {
                        skipped += 1;
                    }
                    continue;
                }
            };

            // ---- session meta (full file, every pass) — every parseable line
            // feeds the accumulator; project_dir deliberately takes cwd from ANY
            // event (cwd rides on user/assistant/system events alike, and
            // subagent or short sessions carry no cwd-bearing system event at
            // all); the accumulator picks the FIRST non-empty one (#83: the
            // launch directory, not the cwd mode).
            meta.observe_ts(ev.timestamp.as_deref());
            meta.observe_cwd(ev.cwd.as_deref());
            // summary / custom_title: latest non-empty wins (see locals above).
            if let Some(s) = &ev.summary {
                let s = s.trim();
                if !s.is_empty() {
                    summary = s.to_string();
                }
            }
            if let Some(ct) = &ev.custom_title {
                let ct = ct.trim();
                if !ct.is_empty() {
                    custom_title = Some(ct.to_string());
                }
            }
            if let Some(m) = &ev.message {
                if m.role.as_deref() == Some("user") {
                    if let Some(t) = first_text_of(m) {
                        // Skip Claude Code command/caveat noise so the
                        // title is the first real prompt, not `/clear`.
                        let t = t.trim();
                        if !t.is_empty()
                            && !t.contains("<local-command-caveat>")
                            && !t.starts_with("<command-name>")
                        {
                            meta.offer_user_title(t);
                        }
                    }
                }
            }

            // ---- incremental (only lines past the cursor) ----
            if line_no <= start_line {
                continue;
            }

            // Transcript messages (trimmed: text + tool_use name; thinking and
            // images dropped; long tool_result/text truncated at TRIM_LIMIT).
            if let Some(m) = &ev.message {
                let ts = ev.timestamp.as_deref().unwrap_or("");
                messages.extend(extract_messages(m, &ev.uuid, &session_id, ts));
            }

            // Per-call usage + per-turn durations (existing message-id dedup).
            let mid = ev.message.as_ref().and_then(|m| m.id.clone());
            match ev.classify(&session_id) {
                Parsed::Usage(u) => {
                    let key = mid.unwrap_or_else(|| u.uuid.clone());
                    events_by_mid
                        .entry(key)
                        .and_modify(|e| {
                            if should_replace(e, &u) {
                                *e = u.clone();
                            }
                        })
                        .or_insert(u);
                }
                Parsed::TurnDuration(td) => turn_durations.push(td),
                Parsed::Skip => {}
            }
        }

        // Session assembly is `finish`'s job (title chain + truncation +
        // saw_any_event → Option<RawSession>); only the per-source differences
        // live here: subagent sessions title from the task description via the
        // `.meta.json` sidecar (the only meaningful name Claude Code gives them —
        // no custom-title/summary events are written for subagents), while main
        // sessions keep the custom-title > summary > first-user-message chain as
        // extra levels. agent_type: `""` for main sessions; the sidecar's agent
        // type (e.g. `Explore`) for subagents, `"agent"` when the sidecar is
        // missing — drives the list's type column.
        let sessions = if is_agent {
            let agent_type = agent_meta
                .as_ref()
                .and_then(|m| m.agent_type.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("agent")
                .to_string();
            // The task description IS the subagent title: present → it wins,
            // absent/blank → forced-empty (a subagent never falls through to
            // the user-message/basename chain).
            let agent_title = agent_meta
                .as_ref()
                .and_then(|m| m.description.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("");
            meta.finish(
                SOURCE_TAG,
                session_id,
                SessionExtras {
                    title_override: Some(agent_title),
                    agent_type: &agent_type,
                    parent_session_id: &parent_session_id,
                    ..Default::default()
                },
            )
            .into_iter()
            .collect()
        } else {
            meta.finish(
                SOURCE_TAG,
                session_id,
                SessionExtras {
                    extra_title_levels: &[custom_title.as_deref().unwrap_or(""), summary.as_str()],
                    ..Default::default()
                },
            )
            .into_iter()
            .collect()
        };

        super::FileParseOutcome {
            events: events_by_mid.into_values().collect(),
            corrections: Vec::new(),
            turn_durations,
            sessions,
            messages,
            skipped,
        }
    }
}

impl SourceParser for ClaudeCodeSourceParser {
    fn name(&self) -> &'static str {
        SOURCE_TAG
    }

    fn discover(&self) -> AppResult<Vec<PathBuf>> {
        // Missing projects dir (no Claude Code sessions on this machine yet)
        // is not an error — the shared skeleton yields no files for an absent
        // root. `agent-*.jsonl` (subagent/sidechain sessions) ARE included:
        // they are real token consumers and show up as subagent sessions with
        // their `.meta.json` task description as title. Without that
        // meta-driven naming they'd flood the list as duplicate-titled noise
        // — the reason they were once skipped (a real project dir had 49
        // agent- files vs 5 real sessions).
        Ok(discover_files(
            &[DirectoryShape {
                root: self.projects_dir.clone(),
                max_depth: None,
            }],
            is_jsonl_file,
        ))
    }

    /// Incremental collect: parse only lines past each file's recorded cursor
    /// and return the advanced cursors to persist. The mtime gate skips
    /// unchanged files (no IO/serde); a never-seen file ({0,0}) falls through to
    /// a full parse on first sight. Delegates to the shared JSONL driver — the
    /// per-file fold is `fold_file`, the same one `parse` uses, so dedup scope
    /// and event classification are identical between the two paths.
    fn collect_incremental(
        &self,
        progress: &ScanProgress,
    ) -> AppResult<(CollectResult, ScanProgressDelta)> {
        collect_jsonl_incremental(self, progress, |file: &Path, text, start_line| {
            Self::fold_file(file, text, start_line)
        })
    }
}

/// Subagent sidecar metadata (`<session_id>.meta.json`), written by Claude Code
/// for every Task subagent: the agent type (e.g. `Explore`) and the task
/// description the user provided. The description becomes the subagent
/// session's title; the type becomes its `agent_type` tag (both are the only
/// naming Claude Code gives subagent sessions — they carry no custom-title or
/// summary events).
#[derive(serde::Deserialize)]
struct AgentMeta {
    #[serde(rename = "agentType", default)]
    agent_type: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

// ---- Lenient session-log deserialization ----
//
// Tolerant by design: every field is optional and unknown fields are ignored,
// so a malformed or schema-drifted line is skipped (counted), never fatal.

#[derive(serde::Deserialize)]
struct SessionEvent {
    #[serde(rename = "type")]
    typ: Option<String>,
    timestamp: Option<String>,
    uuid: Option<String>,
    subtype: Option<String>,
    /// `durationMs` on `system/turn_duration` events.
    #[serde(rename = "durationMs", default)]
    duration_ms: Option<u32>,
    /// `cwd` on user/assistant/system events — the working directory at the
    /// time of the event. The first non-empty one is the session's launch
    /// directory (project_dir); later ones may drift into subdirectories.
    #[serde(default)]
    cwd: Option<String>,
    /// `summary` on a top-level event — Claude's auto-generated session
    /// summary. Latest non-empty value wins (a `/compact` rewrites it later).
    #[serde(default)]
    summary: Option<String>,
    /// `customTitle` on a `type:"custom-title"` event — the user's manual
    /// session name in Claude Code. Latest non-empty wins (a mid-session
    /// rename refreshes the title). Highest-priority title source.
    #[serde(rename = "customTitle", default)]
    custom_title: Option<String>,
    message: Option<ClaudeMessageData>,
}

#[derive(serde::Deserialize)]
struct ClaudeMessageData {
    /// Anthropic message id (e.g. `msg_…`). Shared by every content-block event
    /// of one assistant response — the per-call dedup key (one API call ⇒ one
    /// message id).
    id: Option<String>,
    model: Option<String>,
    usage: Option<SessionUsage>,
    stop_reason: Option<String>,
    /// `user` / `assistant` / ... Used to route transcript extraction.
    #[serde(default)]
    role: Option<String>,
    /// Message body — a string (plain user text) or an array of content blocks
    /// (`text` / `tool_use` / `tool_result` / `thinking` / `image`). Used only
    /// for transcript extraction, never for usage.
    #[serde(default)]
    content: Option<serde_json::Value>,
}

#[derive(serde::Deserialize, Default)]
struct SessionUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    server_tool_use: Option<SessionServerTool>,
    #[serde(default)]
    service_tier: Option<String>,
    /// Iteration entries; we keep only the count (lean).
    #[serde(default)]
    iterations: Option<Vec<serde_json::Value>>,
}

#[derive(serde::Deserialize, Default)]
struct SessionServerTool {
    #[serde(default)]
    web_search_requests: u32,
    #[serde(default)]
    web_fetch_requests: u32,
}

/// How a parsed event should be routed.
enum Parsed {
    Usage(RawUsage),
    TurnDuration(RawTurnDuration),
    Skip,
}

impl SessionEvent {
    /// Classify this event into a usage record, a turn duration, or skip.
    /// `session_id` (the source-log file stem) is stamped onto any emitted
    /// `RawUsage` / `RawTurnDuration` so rows carry their session grouping key.
    fn classify(self, session_id: &str) -> Parsed {
        // Per-turn duration: system event tagged turn_duration.
        if self.typ.as_deref() == Some("system") && self.subtype.as_deref() == Some("turn_duration")
        {
            return match (self.uuid, self.duration_ms) {
                (Some(uuid), Some(duration_ms)) => Parsed::TurnDuration(RawTurnDuration {
                    uuid,
                    timestamp: super::fallback_timestamp(self.timestamp.clone()),
                    session_id: session_id.to_string(),
                    duration_ms,
                }),
                _ => Parsed::Skip,
            };
        }
        // Per-call usage: assistant event with a usable usage block.
        if self.typ.as_deref() == Some("assistant") {
            if let Some(raw) = self.into_usage(session_id) {
                return Parsed::Usage(raw);
            }
        }
        Parsed::Skip
    }

    /// Convert to a `RawUsage` iff this assistant event has a usable usage
    /// block. Drops events with no tokens (e.g. pure tool results).
    fn into_usage(self, session_id: &str) -> Option<RawUsage> {
        let msg = self.message?;
        let usage = msg.usage?;
        let tokens = TokenCounts {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cache_creation: usage.cache_creation_input_tokens,
            cache_read: usage.cache_read_input_tokens,
        };
        // Shared emit gate: all four buckets zero ⇒ no real API usage recorded
        // (e.g. a pure tool-result echo) — never emitted.
        if tokens.is_zero() {
            return None;
        }
        let uuid = self.uuid?;
        let timestamp = super::fallback_timestamp(self.timestamp.clone());
        let st = usage.server_tool_use.unwrap_or_default();
        Some(RawUsage {
            uuid,
            timestamp,
            model: msg.model.unwrap_or_else(|| "unknown".to_string()),
            source: SOURCE_TAG.to_string(),
            session_id: session_id.to_string(),
            tokens,
            server_tool_use: ServerToolUse {
                web_search: st.web_search_requests,
                web_fetch: st.web_fetch_requests,
            },
            stop_reason: msg.stop_reason.unwrap_or_default(),
            service_tier: usage.service_tier.unwrap_or_default(),
            iterations: usage.iterations.map(|v| v.len() as u32).unwrap_or(0),
        })
    }
}

// ---- Transcript message extraction (trimming) ----
//
// Keep text content + tool_use name only; drop thinking blocks' full text and
// base64 images; truncate oversized content at TRIM_LIMIT. One assistant event
// may carry several content blocks → it can emit multiple SessionMessage lines
// (one assistant text message + one tool line per tool_use block).

// The soft cap (TRIM_LIMIT = 32 KiB), the original-title max (TITLE_MAX = 80),
// and the `truncate` helper all live in [`super`] as shared parser helpers,
// so the truncation rule cannot drift between Claude and Codex.

/// First text block of a message's content (string content or the first `text`
/// block of an array). Used for the original-title fallback (first user msg).
fn first_text_of(msg: &ClaudeMessageData) -> Option<String> {
    let content = msg.content.as_ref()?;
    match content {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(blocks) => blocks.iter().find_map(|b| {
            if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                b.get("text").and_then(|v| v.as_str()).map(str::to_string)
            } else {
                None
            }
        }),
        _ => None,
    }
}

/// Extract transcript message lines from one event's message block, stamping
/// `session_id`. Trimming: text content + tool_use name are kept; thinking
/// blocks and image blocks are dropped; long text/tool_result is truncated at
/// [`TRIM_LIMIT`]. Each emitted line gets a stable synthetic uuid (event uuid +
/// block index) so the JSONL append is idempotent across re-collects.
fn extract_messages(
    msg: &ClaudeMessageData,
    event_uuid: &Option<String>,
    session_id: &str,
    ts: &str,
) -> Vec<SessionMessage> {
    let uuid = match event_uuid {
        Some(u) if !u.is_empty() => u.clone(),
        _ => return Vec::new(),
    };
    let content = match msg.content.as_ref() {
        Some(c) => c,
        None => return Vec::new(),
    };
    let role = match msg.role.as_deref() {
        Some(r) => r,
        None => return Vec::new(),
    };
    let mk = |role: SessionMessageRole,
              suffix: &str,
              model: Option<String>,
              name: Option<String>,
              content: String| SessionMessage {
        uuid: format!("{uuid}{suffix}"),
        session_id: session_id.to_string(),
        role,
        ts: ts.to_string(),
        model,
        name,
        content,
    };
    let mut out = Vec::new();
    match role {
        "user" => {
            let texts = collect_text(content);
            let joined = truncate(&texts.join("\n"), TRIM_LIMIT);
            if !joined.is_empty() {
                out.push(mk(SessionMessageRole::User, "", None, None, joined));
            }
        }
        "assistant" => {
            // text blocks → one assistant message; tool_use → per-tool lines.
            let mut text_parts: Vec<String> = Vec::new();
            if let serde_json::Value::Array(blocks) = content {
                for (i, b) in blocks.iter().enumerate() {
                    let t = b.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match t {
                        "text" => {
                            if let Some(txt) = b.get("text").and_then(|v| v.as_str()) {
                                text_parts.push(txt.to_string());
                            }
                        }
                        "tool_use" => {
                            let name = b
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("tool")
                                .to_string();
                            let input = b
                                .get("input")
                                .map(|v| truncate(&v.to_string(), 1024))
                                .unwrap_or_default();
                            out.push(mk(
                                SessionMessageRole::Tool,
                                &format!("#tool{i}"),
                                None,
                                Some(name),
                                input,
                            ));
                        }
                        // thinking / image / unknown → drop (trim noise + bulk).
                        _ => {}
                    }
                }
            } else if let serde_json::Value::String(s) = content {
                text_parts.push(s.clone());
            }
            let joined = truncate(&text_parts.join("\n"), TRIM_LIMIT);
            if !joined.is_empty() {
                out.push(mk(
                    SessionMessageRole::Assistant,
                    "",
                    msg.model.clone(),
                    None,
                    joined,
                ));
            }
        }
        _ => {}
    }
    out
}

/// Collect every `text` block's text from a content value (string or array).
/// `tool_result` content is ignored here (user-role tool_results are dropped to
/// keep the transcript lean; the assistant tool_use line already records the
/// call).
fn collect_text(content: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    match content {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(blocks) => {
            for b in blocks {
                if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                    if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                        out.push(t.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Message-id dedup winner policy: prefer the snapshot with a non-empty
/// stop_reason (the final block of an assistant response); on a tie (both or
/// neither have one) take the larger `output_tokens`. Mirrors CC-Switch's
/// Claude session dedup — a `message_start` snapshot otherwise freezes early
/// and undercounts output.
fn should_replace(existing: &RawUsage, candidate: &RawUsage) -> bool {
    let cand_has_reason = !candidate.stop_reason.is_empty();
    let existing_has_reason = !existing.stop_reason.is_empty();
    if cand_has_reason && !existing_has_reason {
        true
    } else if cand_has_reason == existing_has_reason {
        candidate.tokens.output > existing.tokens.output
    } else {
        false
    }
}

// ---------------------------------------------------------- 测试全量扫面 --
// 昔年 trait 成员 `parse` 的降级（架构审查候选⑪）：test-only、走生产同款驱动
// （Self::fold_file 等 fold 与 `collect_incremental` 共用），但不再是谎称生产的
// trait 接口；需要「显式文件列表全量扫」的测试改走 parse_full。
#[cfg(test)]
impl ClaudeCodeSourceParser {
    pub(crate) fn parse_full(&self, files: &[PathBuf]) -> AppResult<CollectResult> {
        super::parse_jsonl_full(self, files, Self::fold_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    fn write_lines(path: &Path, lines: &[impl AsRef<str>]) {
        let mut f = fs::File::create(path).unwrap();
        for l in lines {
            writeln!(f, "{}", l.as_ref()).unwrap();
        }
    }

    #[test]
    fn parses_assistant_events_and_skips_noise() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session.jsonl");
        let assistant = r#"{"type":"assistant","timestamp":"2026-07-13T16:55:22.467Z","uuid":"abc-1","message":{"model":"glm-5.2","stop_reason":"tool_use","usage":{"input_tokens":100,"output_tokens":50,"cache_read_input_tokens":10,"cache_creation_input_tokens":5,"service_tier":"standard","iterations":[{},{}],"server_tool_use":{"web_search_requests":2}}}}"#;
        let user = r#"{"type":"user","uuid":"abc-2","message":{}}"#;
        write_lines(&file, &[assistant, user, "", "{not json"]);

        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let files = p.discover().unwrap();
        assert_eq!(files.len(), 1);
        let result = p.parse_full(&files).unwrap();
        assert_eq!(result.source, "claude_code");
        assert_eq!(result.events.len(), 1);
        assert!(result.turn_durations.is_empty());
        assert_eq!(result.files_scanned, 1);
        // Only the malformed line counts as skipped: the empty line is ignored,
        // and the user row parses but yields no event (silently dropped).
        assert_eq!(result.lines_skipped, 1);

        let ev = &result.events[0];
        assert_eq!(ev.uuid, "abc-1");
        assert_eq!(ev.model, "glm-5.2");
        assert_eq!(ev.tokens.input, 100);
        assert_eq!(ev.tokens.cache_read, 10);
        assert_eq!(ev.server_tool_use.web_search, 2);
        assert_eq!(ev.stop_reason, "tool_use");
        assert_eq!(ev.service_tier, "standard");
        assert_eq!(ev.iterations, 2);
    }

    #[test]
    fn parses_turn_duration_events() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sess-td.jsonl");
        let td = r#"{"type":"system","subtype":"turn_duration","timestamp":"2026-07-13T16:55:00Z","uuid":"td-1","durationMs":209499}"#;
        let not_td = r#"{"type":"system","subtype":"other","uuid":"x","durationMs":10}"#;
        write_lines(&file, &[td, not_td]);

        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.turn_durations.len(), 1);
        assert_eq!(result.events.len(), 0);
        let td = &result.turn_durations[0];
        assert_eq!(td.uuid, "td-1");
        assert_eq!(td.duration_ms, 209_499);
        assert_eq!(td.timestamp, "2026-07-13T16:55:00Z");
        // The session grouping key rides on the turn too (file stem), so turn
        // aggregates can resolve a project through the sessions table.
        assert_eq!(td.session_id, "sess-td");
    }

    #[test]
    fn drops_assistant_event_with_zero_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let zero = concat!(
            r#"{"type":"assistant","timestamp":"2026-07-13T16:55:22.467Z","uuid":"z","#,
            r#""message":{"model":"glm-5.2","usage":{"input_tokens":0,"output_tokens":0}}}"#
        );
        write_lines(&file, &[zero]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 0);
    }

    #[test]
    fn dedups_assistant_events_by_message_id() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        // One assistant call (msg_A) split into a thinking + a tool_use event,
        // both repeating the full usage; a second call (msg_B) is one event.
        // Distinct message ids must NOT merge.
        let a1 = r#"{"type":"assistant","timestamp":"2026-07-21T15:56:07.000Z","uuid":"u1","message":{"id":"msg_A","model":"glm-5.2","stop_reason":"tool_use","usage":{"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":1000}}}"#;
        let a2 = r#"{"type":"assistant","timestamp":"2026-07-21T15:56:08.000Z","uuid":"u2","message":{"id":"msg_A","model":"glm-5.2","stop_reason":"tool_use","usage":{"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":1000}}}"#;
        let b1 = r#"{"type":"assistant","timestamp":"2026-07-21T16:00:00.000Z","uuid":"u3","message":{"id":"msg_B","model":"glm-5.2","stop_reason":"end_turn","usage":{"input_tokens":200,"output_tokens":20,"cache_read_input_tokens":2000}}}"#;
        write_lines(&file, &[a1, a2, b1]);

        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.events.len(),
            2,
            "msg_A's two content-block events collapse; msg_B stays separate"
        );
        // Deterministic order by timestamp.
        assert_eq!(result.events[0].tokens.input, 100);
        assert_eq!(result.events[0].tokens.cache_read, 1000);
        assert_eq!(result.events[1].tokens.input, 200);
    }

    #[test]
    fn dedup_picks_final_block_over_message_start_snapshot() {
        // One assistant call (msg_A) written as a `message_start` snapshot
        // (output=1, no stop_reason) followed by the final block (full output +
        // stop_reason). The snapshot must NOT win — otherwise output is frozen
        // at 1 and systematically undercounted.
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let start = r#"{"type":"assistant","timestamp":"2026-07-21T15:56:07.000Z","uuid":"u1","message":{"id":"msg_A","model":"glm-5.2","usage":{"input_tokens":100,"output_tokens":1,"cache_read_input_tokens":1000}}}"#;
        let final_block = r#"{"type":"assistant","timestamp":"2026-07-21T15:56:08.000Z","uuid":"u2","message":{"id":"msg_A","model":"glm-5.2","stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":1349,"cache_read_input_tokens":1000}}}"#;
        // Snapshot first, then final.
        write_lines(&file, &[start, final_block]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].tokens.output, 1349);
        assert_eq!(result.events[0].stop_reason, "end_turn");

        // Order-independent: final block first, then a late snapshot — final still wins.
        write_lines(&file, &[final_block, start]);
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].tokens.output, 1349);
        assert_eq!(result.events[0].stop_reason, "end_turn");
    }

    #[test]
    fn discover_on_missing_dir_returns_empty_not_error() {
        let base = tempfile::tempdir().unwrap();
        let p = ClaudeCodeSourceParser::with_dir(base.path().join("does-not-exist"));
        assert!(p.discover().unwrap().is_empty());
    }

    // ---- incremental collect ----

    /// One assistant event line (with message id) for incremental tests.
    fn assistant_line(uuid: &str, mid: &str, out: u32) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-07-21T15:56:07.000Z","uuid":"{uuid}","message":{{"id":"{mid}","model":"glm-5.2","stop_reason":"tool_use","usage":{{"input_tokens":100,"output_tokens":{out}}}}}}}"#
        )
    }

    /// One user event line (plain-text content) for title-chain tests.
    fn user_line(uuid: &str, text: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"2026-07-21T15:55:00.000Z","uuid":"{uuid}","message":{{"role":"user","content":"{text}"}}}}"#
        )
    }

    #[test]
    fn incremental_empty_progress_parses_all_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_lines(&file, &[assistant_line("u1", "msg_A", 10)]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (result, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(result.events.len(), 1, "first run is a full parse");
        assert_eq!(delta.len(), 1, "a cursor is recorded for the file");
        let key = crate::source_parser::scan_progress_key(&file);
        let cursor = delta.get(&key).unwrap();
        assert!(cursor.last_line_offset >= 1);
        assert!(cursor.last_modified > 0);
    }

    #[test]
    fn incremental_skips_unchanged_file_via_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_lines(&file, &[assistant_line("u1", "msg_A", 10)]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.events.len(), 1);
        let progress: ScanProgress = delta;
        // Second collect, file untouched → mtime gate skips it entirely.
        let (r2, delta2) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.events.len(), 0, "unchanged file yields no events");
        assert!(delta2.is_empty(), "unchanged file advances no cursor");
    }

    #[test]
    fn incremental_parses_only_appended_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_lines(&file, &[assistant_line("u1", "msg_A", 10)]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (_, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        // Append a new event — content change bumps mtime past the gate.
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(f, "{}", assistant_line("u2", "msg_B", 20)).unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.events.len(), 1, "only the appended event is parsed");
        assert_eq!(r2.events[0].uuid, "u2");
    }

    #[test]
    fn incremental_truncation_resets_offset() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_lines(
            &file,
            &[
                assistant_line("u1", "msg_A", 10),
                assistant_line("u2", "msg_B", 20),
                assistant_line("u3", "msg_C", 30),
            ],
        );
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (_, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        // Simulate a truncation: rewrite with fewer lines + a new message id.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_lines(&file, &[assistant_line("u9", "msg_NEW", 999)]);
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        // Truncation detected (total < prev offset) → re-read from 0 → the new
        // message is parsed despite the shrunken file.
        assert_eq!(r2.events.len(), 1);
        assert_eq!(r2.events[0].uuid, "u9");
    }

    #[test]
    fn incremental_partial_last_line_not_advanced_past() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let complete = assistant_line("u1", "msg_A", 10);
        // One complete line (with newline) then a partial JSON line WITHOUT a
        // trailing newline — as if Claude is mid-write.
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&file).unwrap();
            writeln!(f, "{complete}").unwrap();
            write!(f, r#"{{"type":"assistant","#).unwrap();
        }
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        let key = crate::source_parser::scan_progress_key(&file);
        let cursor = delta.get(&key).unwrap();
        // 2 lines visible (1 complete + 1 partial), but no trailing newline ⇒
        // cursor stops at line 1, leaving the partial line for next collect.
        assert_eq!(cursor.last_line_offset, 1);
        assert_eq!(r1.events.len(), 1, "complete line parsed, partial skipped");
    }

    /// A session log flushed mid-write can end on a partial multi-byte UTF-8
    /// sequence (an active Chinese session: "中" = E4 B8 AD, only E4 B8
    /// landed). `read_to_string` rejects the WHOLE file on that, so the
    /// session never collected and the cursor never advanced (the `continue`
    /// before `delta.insert`). Lossy reads the complete line before the tail,
    /// skips the partial line, and advances the cursor — the regression pinned
    /// here. See `read_source_lossy` in `super`.
    #[test]
    fn incremental_tolerates_partial_utf8_at_write_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let complete = assistant_line("u1", "msg_A", 10);
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&file).unwrap();
            writeln!(f, "{complete}").unwrap();
            // Partial "中": only the first two of three bytes — invalid UTF-8.
            f.write_all(&[0xE4, 0xB8]).unwrap();
        }
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (result, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(
            result.events.len(),
            1,
            "complete line before the partial tail parses"
        );
        assert_eq!(
            result.sessions.len(),
            1,
            "session meta extracted from the complete line"
        );
        // Cursor recorded + advanced past the complete line. Before the lossy
        // fix, read_to_string errored → `continue` → delta stayed empty.
        let (_, cursor) = delta.iter().next().expect("cursor advanced");
        assert_eq!(
            cursor.last_line_offset, 1,
            "partial last line left for the next collect"
        );
    }

    // ---- session + transcript extraction (Claude only, this phase) ----

    /// A session jsonl yields one RawSession whose id = file stem, whose
    /// project_dir comes from a `cwd` field, whose title_orig comes from
    /// `summary`, and whose started_at/last_active_at bound the timestamps.
    /// Usage events carry the same session_id on their RawUsage.
    #[test]
    fn parses_session_meta_and_stamps_usage_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sess-xyz.jsonl");
        let user = r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u1","cwd":"/home/me/proj","message":{"role":"user","content":"hello world"}}"#;
        let assistant = r#"{"type":"assistant","timestamp":"2026-08-01T11:00:00Z","uuid":"a1","cwd":"/home/me/proj","summary":"Build a thing","message":{"id":"msg_A","model":"glm-5.2","role":"assistant","stop_reason":"end_turn","usage":{"input_tokens":100,"output_tokens":50}}}"#;
        write_lines(&file, &[user, assistant]);

        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();

        // One session, id = file stem.
        assert_eq!(result.sessions.len(), 1);
        let s = &result.sessions[0];
        assert_eq!(s.id, "sess-xyz");
        assert_eq!(s.source, "claude_code");
        assert_eq!(s.project_dir, "/home/me/proj");
        assert_eq!(s.title_orig, "Build a thing"); // summary wins
        assert_eq!(s.started_at, "2026-08-01T10:00:00Z");
        assert_eq!(s.last_active_at, "2026-08-01T11:00:00Z");

        // The usage event carries the session_id.
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].session_id, "sess-xyz");
    }

    /// Title falls back to the first user message text when no `summary`.
    #[test]
    fn session_title_falls_back_to_first_user_message() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let user = r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u1","message":{"role":"user","content":"Please refactor this function for me"}}"#;
        write_lines(&file, &[user]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions.len(), 1);
        assert!(result.sessions[0].title_orig.starts_with("Please refactor"));
        // Truncated at TITLE_MAX (80) with an ellipsis when exceeded.
        let long: String = "x".repeat(200);
        let user2 = format!(
            r#"{{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u2","message":{{"role":"user","content":"{long}"}}}}"#
        );
        write_lines(&file, &[user2]);
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        let t = &result.sessions[0].title_orig;
        assert!(t.ends_with('…'), "long title truncated with ellipsis: {t}");
        assert!(t.chars().count() <= 80);
    }

    /// Transcript messages: text content kept (user + assistant), tool_use →
    /// a separate tool line with the name, thinking/image blocks dropped.
    #[test]
    fn transcript_extraction_keeps_text_and_tool_name_drops_thinking() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let user = r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u1","message":{"role":"user","content":"hi"}}"#;
        let assistant = r#"{"type":"assistant","timestamp":"2026-08-01T10:01:00Z","uuid":"a1","message":{"id":"m1","model":"glm-5.2","role":"assistant","content":[{"type":"thinking","thinking":"internal reasoning"},{"type":"text","text":"Sure"},{"type":"tool_use","name":"Read","input":{"path":"/x"}}]}}"#;
        write_lines(&file, &[user, assistant]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();

        let roles: Vec<_> = result.messages.iter().map(|m| m.role).collect();
        use crate::model::SessionMessageRole::*;
        assert!(roles.contains(&User), "user text kept");
        assert!(roles.contains(&Assistant), "assistant text kept");
        assert!(roles.contains(&Tool), "tool_use → tool line");
        // No thinking content leaked into any message.
        assert!(
            !result
                .messages
                .iter()
                .any(|m| m.content.contains("internal reasoning")),
            "thinking block dropped"
        );
        // Tool line carries the tool name.
        let tool = result.messages.iter().find(|m| m.role == Tool).unwrap();
        assert_eq!(tool.name.as_deref(), Some("Read"));
        // Assistant text is the text block, not the thinking.
        let asst = result
            .messages
            .iter()
            .find(|m| m.role == Assistant)
            .unwrap();
        assert_eq!(asst.content, "Sure");
        assert_eq!(asst.model.as_deref(), Some("glm-5.2"));
    }

    /// Incremental transcript: only lines past the cursor yield messages, but
    /// session meta is rebuilt from the WHOLE file (so started_at survives a
    /// re-collect that only sees the tail).
    #[test]
    fn incremental_messages_only_past_cursor_but_meta_covers_full_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sess-inc.jsonl");
        let u1 = r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u1","message":{"role":"user","content":"first"}}"#;
        write_lines(&file, &[u1]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.messages.len(), 1, "first pass: one message");
        assert_eq!(r1.sessions[0].started_at, "2026-08-01T10:00:00Z");

        // Append a second user line.
        std::thread::sleep(std::time::Duration::from_millis(20));
        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&file)
                .unwrap();
            writeln!(
                f,
                r#"{{"type":"user","timestamp":"2026-08-02T10:00:00Z","uuid":"u2","message":{{"role":"user","content":"second"}}}}"#
            )
            .unwrap();
        }
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        // Only the appended line is a new message.
        assert_eq!(r2.messages.len(), 1);
        assert_eq!(r2.messages[0].content, "second");
        // Meta still covers the full file: started_at from line 1, last_active
        // from line 2.
        assert_eq!(r2.sessions[0].started_at, "2026-08-01T10:00:00Z");
        assert_eq!(r2.sessions[0].last_active_at, "2026-08-02T10:00:00Z");
    }

    /// custom-title wins over summary and the first prompt; a later
    /// custom-title event (a rename mid-session) refreshes the title — the
    /// first-wins bug CC-Switch has.
    #[test]
    fn session_title_prefers_custom_title_and_latest_wins() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let lines = [
            r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u1","message":{"role":"user","content":"first prompt"}}"#,
            r#"{"type":"assistant","timestamp":"2026-08-01T10:01:00Z","uuid":"a1","summary":"Auto summary","message":{"id":"m1","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"{"type":"custom-title","timestamp":"2026-08-01T10:02:00Z","uuid":"c1","customTitle":"Old name"}"#,
            r#"{"type":"custom-title","timestamp":"2026-08-01T10:03:00Z","uuid":"c2","customTitle":"New name"}"#,
        ];
        write_lines(&file, &lines);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions.len(), 1);
        // Latest custom-title wins over summary and the first prompt.
        assert_eq!(result.sessions[0].title_orig, "New name");
    }

    /// summary is latest-seen: a summary emitted later in the file overrides
    /// an earlier one (e.g. a /compact rewrites the summary).
    #[test]
    fn session_summary_latest_wins() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let lines = [
            r#"{"type":"assistant","timestamp":"2026-08-01T10:00:00Z","uuid":"a1","summary":"Early summary","message":{"id":"m1","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"{"type":"assistant","timestamp":"2026-08-01T11:00:00Z","uuid":"a2","summary":"Late summary","message":{"id":"m2","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#,
        ];
        write_lines(&file, &lines);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "Late summary");
    }

    /// First-user-message fallback skips Claude Code command noise — a session
    /// that starts with a `/clear` (`<command-name>`) must title on the first
    /// real prompt, not the command.
    #[test]
    fn session_title_skips_command_noise_for_first_user() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let lines = [
            r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","uuid":"u1","message":{"role":"user","content":[{"type":"text","text":"<command-name>/clear</command-name>"}]}}"#,
            r#"{"type":"user","timestamp":"2026-08-01T10:01:00Z","uuid":"u2","message":{"role":"user","content":"Refactor the parser"}}"#,
        ];
        write_lines(&file, &lines);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert!(
            result.sessions[0]
                .title_orig
                .starts_with("Refactor the parser"),
            "command noise skipped: {}",
            result.sessions[0].title_orig
        );
    }

    /// With no custom-title, summary, or user message, the title falls back to
    /// the project dir basename (cwd's last path segment); project_dir keeps
    /// the full cwd.
    #[test]
    fn session_title_falls_back_to_project_dir_basename() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        // An assistant event with usage but no summary and no user message →
        // only cwd is available for the title.
        let line = r#"{"type":"assistant","timestamp":"2026-08-01T10:00:00Z","uuid":"a1","cwd":"/home/me/O_cc one","message":{"id":"m1","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#;
        write_lines(&file, &[line]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].title_orig, "O_cc one");
        assert_eq!(result.sessions[0].project_dir, "/home/me/O_cc one");
    }

    // ---- project_dir = first non-empty cwd ----

    /// The acceptance case of issue #83: a session starts at the repo root,
    /// then works inside a subdirectory for most of its life. project_dir
    /// must be the FIRST cwd (the launch directory), not the cwd mode — the
    /// mode is what pinned such sessions to `…\src-tauri` buckets.
    #[test]
    fn session_project_dir_uses_first_cwd_not_mode() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let root = r#"{"type":"user","timestamp":"2026-08-01T09:00:00Z","uuid":"u1","cwd":"D:\\Project\\O_CC_One","message":{"role":"user","content":"hi"}}"#;
        let sub1 = r#"{"type":"assistant","timestamp":"2026-08-01T10:00:00Z","uuid":"a1","cwd":"D:\\Project\\O_CC_One\\src-tauri","message":{"id":"m1","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#;
        let sub2 = r#"{"type":"assistant","timestamp":"2026-08-01T11:00:00Z","uuid":"a2","cwd":"D:\\Project\\O_CC_One\\src-tauri","message":{"id":"m2","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#;
        write_lines(&file, &[root, sub1, sub2]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(
            result.sessions[0].project_dir, "D:\\Project\\O_CC_One",
            "first cwd (launch dir) wins even when the subdir is the mode (2×)"
        );
    }

    /// Empty/whitespace cwd values never become project_dir — the first
    /// NON-empty cwd is picked; a file with no usable cwd at all degrades to
    /// an empty project_dir (the same fallback the mode picker had).
    #[test]
    fn session_project_dir_skips_empty_cwd_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        let empty_cwd = r#"{"type":"user","timestamp":"2026-08-01T09:00:00Z","uuid":"u1","cwd":"  ","message":{"role":"user","content":"hi"}}"#;
        let real = r#"{"type":"user","timestamp":"2026-08-01T09:01:00Z","uuid":"u2","cwd":"/home/me/proj","message":{"role":"user","content":"again"}}"#;
        write_lines(&file, &[empty_cwd, real]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].project_dir, "/home/me/proj");

        let no_cwd = r#"{"type":"assistant","timestamp":"2026-08-01T09:02:00Z","uuid":"a1","message":{"id":"m1","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#;
        write_lines(&file, &[no_cwd]);
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].project_dir, "");
    }

    /// The reconcile "seen" set comes from the DISCOVERED FILES, not the parsed
    /// sessions — the mtime gate skips unchanged files, so a seen set derived
    /// from parsed output would empty out on a no-change collect and wipe every
    /// real session as a ghost. Regression test for exactly that trap.
    #[test]
    fn session_ids_seen_survives_the_mtime_gate() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.jsonl");
        write_lines(&file, &[assistant_line("u1", "msg_A", 10)]);
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(
            r1.session_ids,
            vec!["s".to_string()],
            "first pass sees the file's session"
        );
        // Second collect: file unchanged → mtime gate skips it entirely.
        let (r2, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(
            r2.sessions.len(),
            0,
            "no parsed output on a no-change collect"
        );
        assert_eq!(
            r2.session_ids,
            vec!["s".to_string()],
            "seen set still covers the discovered file — never derived from parsed output"
        );
    }

    /// Agent sub-session files are part of the seen set too — they are real
    /// sessions now (subagent sessions with `.meta.json` naming), so their
    /// stale rows must NOT be reconciled away.
    #[test]
    fn session_ids_seen_includes_agent_files() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        write_lines(
            &proj.join("249e8e6b.jsonl"),
            &[assistant_line("u1", "msg_A", 10)],
        );
        write_lines(
            &proj.join("agent-a10c476b.jsonl"),
            &[assistant_line("u2", "msg_B", 20)],
        );
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let files = p.discover().unwrap();
        assert_eq!(files.len(), 2, "discover includes agent files");
        // discover 的 walkdir 顺序平台依赖（Linux ext4 与 Windows 不同），seen
        // 集合只论成员资格——排序后断言，与生产路径 collect 对 session_ids 的
        // 排序同精神。
        let mut seen = p.session_ids_seen(&files);
        seen.sort_unstable();
        assert_eq!(
            seen,
            vec!["249e8e6b".to_string(), "agent-a10c476b".to_string()],
            "seen set matches discover — agent ids included"
        );
    }

    /// `agent-*.jsonl` are Claude Code subagent/sidechain sessions — discover
    /// includes them (they consume real tokens and get a `.meta.json`-driven
    /// title), so the list shows them as subagent sessions instead of dropping
    /// their usage entirely.
    #[test]
    fn discover_includes_agent_subagent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        // One real user session + two subagent sidechain files.
        write_lines(
            &proj.join("249e8e6b.jsonl"),
            &[assistant_line("u1", "msg_A", 10)],
        );
        write_lines(
            &proj.join("agent-a10c476b.jsonl"),
            &[assistant_line("u2", "msg_B", 20)],
        );
        write_lines(
            &proj.join("agent-a1366047.jsonl"),
            &[assistant_line("u3", "msg_C", 30)],
        );
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let files = p.discover().unwrap();
        assert_eq!(files.len(), 3, "agent- subagent files included");
    }

    /// A subagent file + its `.meta.json` fold into one session carrying the
    /// agent type tag and the task description as title; a subagent without
    /// the sidecar degrades to the generic `"agent"` tag. Main sessions keep
    /// `agent_type == ""` and their regular title chain.
    #[test]
    fn fold_subagent_session_from_meta_json() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        // Subagent with sidecar meta: type + description.
        write_lines(
            &proj.join("agent-aaa.jsonl"),
            &[assistant_line("u1", "msg_A", 10)],
        );
        fs::write(
            proj.join("agent-aaa.meta.json"),
            r#"{"agentType": "Explore", "description": "核实 cc-switch 供应商"}"#,
        )
        .unwrap();
        // Subagent without sidecar: generic "agent" tag, no title — even when
        // the file carries cwd lines, it never falls through to the
        // main-session title chain.
        write_lines(
            &proj.join("agent-bbb.jsonl"),
            &[
                assistant_line("u2", "msg_B", 20),
                r#"{"type":"assistant","timestamp":"2026-08-01T10:00:00Z","uuid":"u5","cwd":"/tmp/agent-proj","message":{"id":"m9","model":"glm-5.2","role":"assistant","usage":{"input_tokens":1,"output_tokens":1}}}"#.to_string(),
            ],
        );
        // Main session: empty agent_type, regular title chain (first user msg).
        write_lines(
            &proj.join("main.jsonl"),
            &[
                user_line("u3", "hello world"),
                assistant_line("u4", "msg_C", 30),
            ],
        );
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let outcome = p.parse_full(&p.discover().unwrap()).unwrap();

        let by_id = |id: &str| {
            outcome
                .sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("session {id} missing"))
        };
        let sub = by_id("agent-aaa");
        assert_eq!(sub.agent_type, "Explore");
        assert_eq!(sub.title_orig, "核实 cc-switch 供应商");
        let bare = by_id("agent-bbb");
        assert_eq!(bare.agent_type, "agent");
        assert_eq!(bare.title_orig, "");
        let main = by_id("main");
        assert_eq!(main.agent_type, "");
        assert_eq!(main.title_orig, "hello world");
        // Subagent usage is parsed like any other session's.
        assert!(outcome.events.iter().any(|u| u.session_id == "agent-aaa"));
    }

    /// Parent link (#90): Claude Code writes subagent files at
    /// `<project>/<parent-session-id>/subagents/agent-*.jsonl` — the directory
    /// chain is the EXPLICIT parent field, so a nested agent file carries its
    /// parent's id, while the legacy flat layout (agent files directly in the
    /// project dir) and main sessions carry `""` (no heuristic inference).
    #[test]
    fn fold_derives_parent_from_subagents_directory_placement() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("proj");
        let nested = proj
            .join("196aa2c3-ee4f-4408-929d-b356c7dbb25c")
            .join("subagents");
        fs::create_dir_all(&nested).unwrap();
        // Nested subagent (the current Claude Code layout) + its parent main
        // session file at the project root.
        write_lines(
            &nested.join("agent-a01d98.jsonl"),
            &[assistant_line("u1", "msg_A", 10)],
        );
        write_lines(
            &proj.join("196aa2c3-ee4f-4408-929d-b356c7dbb25c.jsonl"),
            &[assistant_line("u2", "msg_B", 20)],
        );
        // Legacy flat subagent: same project dir, no subagents/ wrapper.
        write_lines(
            &proj.join("agent-flat.jsonl"),
            &[assistant_line("u3", "msg_C", 30)],
        );
        let p = ClaudeCodeSourceParser::with_dir(dir.path().to_path_buf());
        let outcome = p.parse_full(&p.discover().unwrap()).unwrap();

        let by_id = |id: &str| {
            outcome
                .sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap_or_else(|| panic!("session {id} missing"))
        };
        assert_eq!(
            by_id("agent-a01d98").parent_session_id,
            "196aa2c3-ee4f-4408-929d-b356c7dbb25c",
            "nested placement names the parent session"
        );
        assert_eq!(
            by_id("agent-flat").parent_session_id,
            "",
            "flat layout carries no link — never guessed"
        );
        assert_eq!(
            by_id("196aa2c3-ee4f-4408-929d-b356c7dbb25c").parent_session_id,
            "",
            "a main session has no parent"
        );
    }
}
