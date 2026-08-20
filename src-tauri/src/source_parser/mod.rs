//! Source-log parsers (parse local session logs).
//!
//! Plugin trait + shared incremental driver. Concrete parsers live in
//! submodules (`claude`, `codex`, `gemini`, `grok`, `opencode`). Discovery
//! funnels through one shared directory-walk skeleton ([`DirectoryShape`] +
//! [`discover_files`]) and each file's incremental gate is declared
//! ([`GateMode`] via [`SourceParser::gate_mode`]) — parsers never inline
//! their own walkers or pretend a line cursor they don't honor. A parser
//! discovers Source files and parses them into two raw streams:
//!   - per-call [`RawUsage`] (one per `assistant` event = one API request), and
//!   - per-turn [`RawTurnDuration`] (from `system/turn_duration` events).
//!
//! Both are pre-device / pre-cost — the parser does NOT know about deviceId
//! or pricing. That is applied by the ingest layer, so the same parser output
//! can land in the Local Store (Standalone) and the JSONL Artifact.

use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::model::{RawSession, ServerToolUse, SessionMessage, TokenCounts};

mod claude;
mod codex;
mod gemini;
mod grok;
mod opencode;

/// A single parsed per-call usage event (parser output, pre-cost / pre-device).
///
/// `Default` is the empty-record shape the append parsers use via
/// `..Default::default()` for the tail fields sources don't populate
/// (server_tool_use / stop_reason / service_tier / iterations) — the
/// one place that zero tail is written.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawUsage {
    /// Globally-unique id from the Source log — the dedup key.
    pub uuid: String,
    /// ISO8601 UTC timestamp from the Source log.
    pub timestamp: String,
    /// Billed / mapped model string, e.g. `glm-5.2`.
    pub model: String,
    /// Source tag, e.g. `claude_code`.
    pub source: String,
    /// Session this call belongs to (the source log's session identifier).
    /// All five parsers wire sessions. NOT part of the dedup key — grouping
    /// info only.
    pub session_id: String,
    pub tokens: TokenCounts,
    pub server_tool_use: ServerToolUse,
    /// Semantic termination reason (`tool_use` / `end_turn` / …). NOT an HTTP status.
    pub stop_reason: String,
    /// Service tier label, e.g. `standard`.
    pub service_tier: String,
    /// Reasoning/thinking iteration count (source array length).
    pub iterations: u32,
}

/// A single parsed per-turn duration (parser output, pre-device). Sourced from
/// the `system/turn_duration` event's `durationMs`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTurnDuration {
    /// Dedup key (the source event's uuid).
    pub uuid: String,
    pub timestamp: String,
    /// Session this turn belongs to (the source log's session identifier) —
    /// the grouping key that lets turn aggregates resolve a project, exactly
    /// like `RawUsage::session_id`.
    pub session_id: String,
    /// Turn wall-clock in milliseconds.
    pub duration_ms: u32,
}

/// Outcome of parsing one parser's sources.
#[derive(Debug, Clone, Default)]
pub struct CollectResult {
    pub source: String,
    pub events: Vec<RawUsage>,
    /// Correction candidates (Codex unknown-model self-heal — parser half):
    /// events an EARLIER pass already wrote with `model = "unknown"` (rows at
    /// or before the scan cursor), re-emitted this pass with the model now
    /// that a `turn_context` / `info.model` line resolved it. Carried in a
    /// dedicated channel — NOT `events` — so the ingest layer routes them
    /// through the guarded upsert that rewrites exactly the store rows that
    /// still read `model = 'unknown'` (the protocol's store half, see
    /// `Store::ingest_corrections_marking_dirty`). The parser re-offers them
    /// on EVERY pass (it cannot tell which pre-model rows an earlier pass
    /// wrote), and the guard turns re-offers into no-ops once a row carries
    /// the model. Other sources never populate this.
    pub corrections: Vec<RawUsage>,
    /// Per-turn durations (from `system/turn_duration` events).
    pub turn_durations: Vec<RawTurnDuration>,
    /// Sessions discovered this pass (system data: id/source/project/title/
    /// timestamps). One per source log file (Claude: one session per jsonl).
    pub sessions: Vec<RawSession>,
    /// Transcript messages extracted this pass (only the new lines past each
    /// file's cursor). Collected for ALL sessions so the ingest layer can decide
    /// per-session (only favorited ones land in `sessions/<id>.jsonl`).
    pub messages: Vec<SessionMessage>,
    /// Files scanned.
    pub files_scanned: u32,
    /// Lines that failed to parse (skipped, not fatal).
    pub lines_skipped: u32,
    /// Session ids SEEN this pass, derived from the DISCOVERED FILES — not
    /// from the parsed `sessions` (the mtime gate skips unchanged files, so the
    /// parsed set would shrink to zero on a no-change collect and treat every
    /// real session as a ghost). Powers the sessions-table reconciliation at
    /// ingest: rows in `(device_id, source)` not in this set are deleted.
    /// Empty for parsers without file-backed sessions (opencode), which
    /// disables reconciliation for them.
    pub session_ids: Vec<String>,
}

/// Per-file incremental scan cursor. Persisted in `scan_progress`;
/// replaceable — a lost cursor triggers a full rescan (the store's
/// `(uuid, device_id)` ingest dedup absorbs the re-read rows).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileCursor {
    /// File mtime (nanos) as last seen by this cursor.
    pub last_modified: i64,
    /// Last fully-processed 1-based line number. 0 = nothing parsed yet.
    /// Mtime-only files ([`GateMode::MtimeOnly`]) keep this at 0 — they have
    /// no line cursor.
    pub last_line_offset: i64,
}

/// file_path → cursor. Loaded before `collect_incremental`, saved after. A plain
/// `HashMap` alias (not a newtype) — it is a trivial wrapper.
pub type ScanProgress = std::collections::HashMap<String, FileCursor>;

/// One collect's worth of cursor advances: only entries for files actually
/// opened and read. Saved as an UPSERT. Same shape as `ScanProgress` (a subset).
pub type ScanProgressDelta = std::collections::HashMap<String, FileCursor>;

/// Source-parser plugin interface (extensible to Codex / Gemini / …).
pub trait SourceParser: Send + Sync {
    /// Stable parser tag, e.g. `claude_code`. Becomes `RawUsage.source`.
    fn name(&self) -> &'static str;

    /// Discover Source files for this parser.
    fn discover(&self) -> AppResult<Vec<PathBuf>>;

    /// Full-scan parse of the given files into usage events + turn durations.
    /// Diagnostic/test surface and the semantic reference for a "parse
    /// everything" run; the production collect path is
    /// [`SourceParser::collect_incremental`] (which, with empty progress, also
    /// yields a full scan). Each parser's `parse` delegates to the same
    /// parsing helpers its `collect_incremental` closure uses, so testing via
    /// `parse` exercises production logic — not a divergent path.
    #[allow(dead_code)] // off the production path by design; kept as the test/diagnostic full-scan surface
    fn parse(&self, files: &[PathBuf]) -> AppResult<CollectResult>;

    /// Incremental collect: parse only what each file's recorded cursor says is
    /// new, returning the advanced cursors to persist. This is the ONLY collect
    /// entry point the production path (`collect_into`) calls. Each parser
    /// implements its own — the JSONL parsers delegate to the shared
    /// [`collect_jsonl_incremental`] driver; OpenCode (SQLite, two-level
    /// watermark) keeps its own. There is intentionally no default impl: a
    /// parser that skipped this would have no production collect path.
    fn collect_incremental(
        &self,
        progress: &ScanProgress,
    ) -> AppResult<(CollectResult, ScanProgressDelta)>;

    /// Incremental gate strategy for one of this parser's source files —
    /// declared, not assumed, so [`collect_jsonl_incremental`] never pretends
    /// a line cursor for files that don't honor it (Gemini's single-JSON
    /// files, Grok summary.json) and SQLite parsers (OpenCode) state their
    /// watermark model. The driver consults this per file; the default is the
    /// append-JSONL line cursor.
    fn gate_mode(&self, _file: &Path) -> GateMode {
        GateMode::LineCursor
    }

    /// Session ids represented by the discovered files — the reconciliation
    /// "seen" set. Default = file stem (Claude: one session per jsonl and the
    /// id IS the stem; discover already filters `agent-*`, so stale agent rows
    /// clear on the first reconciled collect). Parsers whose session id lives
    /// INSIDE the file (Codex thread id, Gemini `sessionId`) or in the path
    /// shape (Grok parent-dir name) MUST override — the stem default would
    /// mis-delete real sessions. Parsers without file-backed sessions
    /// (opencode, SQLite-managed) override with an empty set to disable
    /// reconciliation entirely.
    fn session_ids_seen(&self, files: &[PathBuf]) -> Vec<String> {
        files
            .iter()
            .filter_map(|f| f.file_stem().and_then(|s| s.to_str()))
            .map(str::to_string)
            .collect()
    }
}

/// All enabled Source-log parsers, in collection order, rooted at the real
/// home dir. A parser whose source dir is absent simply discovers no files
/// (not an error), so every parser is always instantiated; the shared
/// `scan_progress` table keys by file path, which is naturally disjoint across
/// parsers.
pub fn all_source_parsers() -> AppResult<Vec<Box<dyn SourceParser>>> {
    let home = dirs::home_dir()
        .ok_or_else(|| crate::error::AppError::SourceParser("cannot resolve home dir".into()))?;
    Ok(all_source_parsers_at(&home))
}

/// Root-injection seam for the collect orchestration: every enabled parser
/// rooted at `home` (production goes through [`all_source_parsers`]; the
/// collect integration test passes a tempdir fixture). `collect_into_with`
/// runs its full orchestration against whatever this returns, so the collect
/// invariants (cursors saved only after all ingests) are testable without a
/// real `~/.claude`.
pub fn all_source_parsers_at(home: &Path) -> Vec<Box<dyn SourceParser>> {
    vec![
        Box::new(claude::ClaudeCodeSourceParser::new_at(home)),
        Box::new(codex::CodexSourceParser::new_at(home)),
        Box::new(gemini::GeminiCliSourceParser::new_at(home)),
        Box::new(grok::GrokSourceParser::new_at(home)),
        Box::new(opencode::OpenCodeSourceParser::new_at(home)),
    ]
}

/// One JSONL file's parse result. The parser's per-file parser returns this;
/// the shared incremental driver below handles everything else (mtime gate,
/// truncation self-heal, partial-last-line guard, cursor advance, ordering).
pub(super) struct FileParseOutcome {
    pub(super) events: Vec<RawUsage>,
    /// Correction candidates for this file (see [`CollectResult::corrections`]).
    /// Only the Codex parser populates it.
    pub(super) corrections: Vec<RawUsage>,
    pub(super) turn_durations: Vec<RawTurnDuration>,
    /// Sessions discovered in this file (one per Claude jsonl; empty for
    /// parsers not yet wired for sessions).
    pub(super) sessions: Vec<RawSession>,
    /// Transcript messages parsed this pass (only lines past the cursor).
    pub(super) messages: Vec<SessionMessage>,
    pub(super) skipped: u32,
}

/// How a parser's per-file incremental gate advances — declared per file via
/// [`SourceParser::gate_mode`], so the line-cursor contract of
/// [`collect_jsonl_incremental`] is never pretended for files that don't
/// honor one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateMode {
    /// Skip lines at/before the stored `last_line_offset`; the driver hands
    /// `parse_file` the 1-based start line (self-healed on truncation) and
    /// advances the offset (partial-last-line guard). Append-only JSONL logs:
    /// Claude, Codex, Grok chat/updates.
    LineCursor,
    /// Re-parse the whole file whenever it passes the mtime gate; the line
    /// offset is meaningless and stays 0. Single-object JSON: Gemini,
    /// Grok summary.json.
    MtimeOnly,
    /// Per-session watermark inside the source (SQLite: OpenCode). The shared
    /// JSONL driver does not apply.
    SessionWatermark,
}

/// Shared incremental collect for line-oriented JSONL sources. Walks every
/// discovered file: mtime-gates unchanged ones, re-reads changed ones, and
/// hands the file text + start line to `parse_file`. Each file's [`GateMode`]
/// (declared by its parser via [`SourceParser::gate_mode`]) decides whether
/// `start_line` is meaningful: line-cursor files (Claude, Codex, Grok
/// chat/updates) skip lines at or before it (already self-healed on
/// truncation); mtime-only files (Gemini's single JSON object, Grok
/// summary.json) ignore it and re-parse the whole text on each gate pass,
/// recording no line offset. OpenCode (SQLite, two-level session watermark)
/// keeps its own `collect_incremental` — its source shape does not fit this
/// driver.
pub(super) fn collect_jsonl_incremental(
    parser: &dyn SourceParser,
    progress: &ScanProgress,
    parse_file: impl Fn(&Path, &str, i64) -> FileParseOutcome,
) -> AppResult<(CollectResult, ScanProgressDelta)> {
    let files = parser.discover()?;
    let mut events: Vec<RawUsage> = Vec::new();
    let mut corrections: Vec<RawUsage> = Vec::new();
    let mut turn_durations: Vec<RawTurnDuration> = Vec::new();
    let mut sessions: Vec<RawSession> = Vec::new();
    let mut messages: Vec<SessionMessage> = Vec::new();
    let mut skipped = 0u32;
    let mut delta = ScanProgressDelta::new();

    for file in &files {
        let gate_mode = parser.gate_mode(file);
        let path_str = scan_progress_key(file);
        // mtime gate — one stat; unchanged files do no IO/serde.
        let metadata = match std::fs::metadata(file) {
            Ok(m) => m,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let mtime = metadata_modified_nanos(&metadata);
        let prev = progress.get(&path_str).copied().unwrap_or_default();
        // `prev.last_modified != 0` lets a never-seen file parse in full.
        if prev.last_modified != 0 && mtime <= prev.last_modified {
            continue;
        }
        let text = match read_source_lossy(file) {
            Some(t) => t,
            None => {
                skipped += 1;
                continue;
            }
        };
        // Line-cursor files derive the start line (truncation self-heal) and
        // the advanced offset (partial-last-line guard); mtime-only files get
        // (0, 0) — the offset is meaningless for them and stays 0.
        let (start_line, new_offset) = if gate_mode == GateMode::LineCursor {
            let total_lines = text.lines().count() as i64;
            // Truncation self-heal: if the file shrank below the last known
            // offset, re-read from the start (would otherwise silently drop
            // post-truncation appends).
            let start_line = if total_lines < prev.last_line_offset {
                0
            } else {
                prev.last_line_offset
            };
            // Partial-last-line guard: no trailing newline ⇒ the last line may
            // be mid-write; don't advance past it or the next collect skips it.
            let ends_clean = text.ends_with('\n') || text.ends_with('\r');
            let new_offset = if ends_clean {
                total_lines
            } else if total_lines > start_line {
                total_lines - 1
            } else {
                start_line
            };
            (start_line, new_offset)
        } else {
            (0, 0)
        };
        let outcome = parse_file(file, &text, start_line);
        events.extend(outcome.events);
        corrections.extend(outcome.corrections);
        turn_durations.extend(outcome.turn_durations);
        sessions.extend(outcome.sessions);
        messages.extend(outcome.messages);
        skipped += outcome.skipped;
        delta.insert(
            path_str,
            FileCursor {
                last_modified: mtime,
                last_line_offset: new_offset,
            },
        );
    }

    // Deterministic order (shared with `parse_jsonl_full`): events and
    // corrections by (timestamp, uuid), sessions by (last_active_at, id).
    order_results(&mut events, &mut sessions, &mut corrections);
    // Messages: keep source order (within-file chronological); cross-file extend
    // is stable per the discover() order, which is deterministic per OS but not
    // sorted — the ingest layer re-groups by session_id before writing, and each
    // session's file is appended in this order.
    // files_scanned stays "discovered count" — do not redefine to "parsed count".
    // session_ids come from the FILES, not the parsed sessions — the mtime gate
    // would otherwise empty them on a no-change collect (see the field docs).
    // The seen set is order-free for reconcile (membership only), but sort it
    // anyway — discover's walkdir order is platform-dependent, and a
    // deterministic order keeps collected results stable (same spirit as
    // `order_results` above).
    let mut session_ids = parser.session_ids_seen(&files);
    session_ids.sort_unstable();
    let result = CollectResult {
        source: parser.name().to_string(),
        events,
        corrections,
        turn_durations,
        sessions,
        messages,
        files_scanned: files.len() as u32,
        lines_skipped: skipped,
        session_ids,
    };
    Ok((result, delta))
}

/// Deterministic ordering for collected results: events and corrections by
/// (timestamp, uuid), sessions by (last_active_at, id). Applied by every
/// parse/collect path so repeated runs over the same sources yield identical
/// grain lines — the single rule, replacing the sort copy each parser used to
/// inline.
pub(super) fn order_results(
    events: &mut [RawUsage],
    sessions: &mut [RawSession],
    corrections: &mut [RawUsage],
) {
    events.sort_by(|a, b| (&a.timestamp, &a.uuid).cmp(&(&b.timestamp, &b.uuid)));
    corrections.sort_by(|a, b| (&a.timestamp, &a.uuid).cmp(&(&b.timestamp, &b.uuid)));
    sessions.sort_by(|a, b| (&a.last_active_at, &a.id).cmp(&(&b.last_active_at, &b.id)));
}

/// Full-scan parse for line-oriented JSONL sources — the shared "parse every
/// discovered file in full" loop that each JSONL parser's `parse` used to
/// inline. Hands each file's text to `parse_file` (start_line 0 — full scan),
/// aggregates events / turn durations / sessions / messages, and applies the
/// deterministic ordering. The only per-parser thing is "how a file's text
/// becomes events", supplied as the same `parse_file` closure the parser's
/// `collect_incremental` uses — so the test path (`parse`) and the production
/// path (`collect_incremental`) run identical per-file logic. OpenCode (SQLite)
/// keeps its own `parse`; its source shape does not fit this driver.
pub(super) fn parse_jsonl_full(
    parser: &dyn SourceParser,
    files: &[PathBuf],
    parse_file: impl Fn(&Path, &str, i64) -> FileParseOutcome,
) -> AppResult<CollectResult> {
    let mut events = Vec::new();
    let mut corrections = Vec::new();
    let mut turn_durations = Vec::new();
    let mut sessions = Vec::new();
    let mut messages = Vec::new();
    let mut skipped = 0u32;
    for file in files {
        let text = match read_source_lossy(file) {
            Some(t) => t,
            None => {
                skipped += 1;
                continue;
            }
        };
        let outcome = parse_file(file, &text, 0);
        events.extend(outcome.events);
        corrections.extend(outcome.corrections);
        turn_durations.extend(outcome.turn_durations);
        sessions.extend(outcome.sessions);
        messages.extend(outcome.messages);
        skipped += outcome.skipped;
    }
    order_results(&mut events, &mut sessions, &mut corrections);
    let mut session_ids = parser.session_ids_seen(files);
    session_ids.sort_unstable();
    Ok(CollectResult {
        source: parser.name().to_string(),
        events,
        corrections,
        turn_durations,
        sessions,
        messages,
        files_scanned: files.len() as u32,
        lines_skipped: skipped,
        session_ids,
    })
}

/// One directory shape a parser declares for discovery: a root to walk plus
/// the max directory depth at which files are collected.
pub(super) struct DirectoryShape {
    pub(super) root: PathBuf,
    /// Max depth below `root` (walkdir depth: `root` = 0, its children = 1)
    /// at which files are collected; directories at exactly `max_depth` are
    /// not descended into. `None` = unlimited.
    pub(super) max_depth: Option<u32>,
}

/// The shared directory-walk skeleton behind every file-backed parser's
/// `discover` — the traversal invariants live here once, not in per-parser
/// copies:
///   - a missing/empty root yields no files (absent source dir is not an
///     error);
///   - only regular files are collected, in deterministic order per call
///     (readdir order, never sorted — the collect layer sorts what needs
///     sorting);
///   - symlinks are never followed (not even symlinked files);
///   - unreadable subtrees are skipped entry-wise.
///
/// Per-parser code only declares its directory shapes and a filename
/// predicate.
pub(super) fn discover_files(
    shapes: &[DirectoryShape],
    is_target: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for shape in shapes {
        if !shape.root.is_dir() {
            continue; // absent source dir — not an error
        }
        for entry in walkdir::WalkDir::new(&shape.root)
            .follow_links(false)
            .max_depth(shape.max_depth.unwrap_or(u32::MAX) as usize)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if is_target(entry.path()) {
                out.push(entry.path().to_path_buf());
            }
        }
    }
    out
}

/// Shared filename predicate for the append-JSONL parsers (Claude, Codex).
pub(super) fn is_jsonl_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()) == Some("jsonl")
}

/// Normalize a cache-inclusive `input` — one whose value already contains its
/// `cache_read` portion — into cc one's fresh-input representation: subtract
/// `cache_read` (floored at 0) and clamp `cache_read` so it can never exceed
/// `input`. Returns `(fresh_input, clamped_cache_read)`.
///
/// Cache-inclusive sources (Codex, Gemini, Grok) call this at parse time so the
/// `RawUsage.input` they emit is always fresh — the one hard contract every
/// parser must satisfy. Fresh sources (Claude, OpenCode) carry fresh input
/// natively and skip this step.
pub(super) fn normalize_cache_inclusive(input: u32, cache_read: u32) -> (u32, u32) {
    let clamped = cache_read.min(input);
    let fresh = input.saturating_sub(clamped);
    (fresh, clamped)
}

/// Soft cap on any single content string (32 KiB). Oversized content is
/// truncated; a per-session 5 MiB soft cap is warned-on only at the ingest
/// layer, not enforced here. Shared across JSONL parsers so the rule cannot
/// drift between them (single source of truth).
pub(super) const TRIM_LIMIT: usize = 32 * 1024;

/// Max chars of the original title (summary or first user message). Shared
/// across session-emitting parsers.
pub(super) const TITLE_MAX: usize = 80;

/// Truncate a string to `limit` chars, appending an ellipsis when shortened.
/// Shared across parsers — one truncation rule so Claude / Codex / … cannot
/// silently diverge (architecture review: single source of truth).
pub(super) fn truncate(s: &str, limit: usize) -> String {
    if s.chars().count() <= limit {
        return s.to_string();
    }
    let head: String = s.chars().take(limit.saturating_sub(1)).collect();
    format!("{head}…")
}

/// File mtime in nanos since UNIX_EPOCH, for the incremental mtime gate. Clamped
/// to `i64::MAX` (the SQLite column is INTEGER). Returns 0 if mtime is
/// unavailable — then the gate never skips (safe, just re-parses).
pub(super) fn metadata_modified_nanos(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Read a source-log file as text, tolerating a partial multi-byte UTF-8
/// sequence at the write boundary. An active session being appended to may be
/// flushed mid-character; `read_to_string` rejects the WHOLE file on that, so
/// the session's meta/transcript never land and the scan cursor never advances
/// (see `collect_jsonl_incremental`). Reading bytes + lossy decode turns the
/// truncated tail into U+FFFD: the line holding it fails JSON parsing and is
/// counted as `skipped` like any malformed line, while every complete line
/// before it parses normally. Returns `None` only on a real IO error (file
/// vanished mid-scan) — the caller then skips the file.
pub(super) fn read_source_lossy(file: &Path) -> Option<String> {
    let bytes = std::fs::read(file).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// 缺时间戳的回填策略（单一归属，#71）：原始记录没带时间戳 → 用采集时刻。
/// 已知偏差：回填的记录被分桶到采集当天而非真实发生日——策略集中在此，
/// 「回填日 vs 未知」的正确呈现可单独讨论后在这一处替换。
pub(super) fn fallback_timestamp(ts: Option<String>) -> String {
    ts.unwrap_or_else(crate::time::now_iso)
}

/// 读文件头的有界前缀（64 KiB）为 UTF-8 文本——session 元数据（sessionId /
/// session_meta）都在文件顶部，ghost 回收核对身份时整文件读取太浪费。任何
/// 失败（打不开 / 读错 / 非 UTF-8）→ 空串：调用方回退到按文件名解析，回退
/// 只会多保留一行、绝不误删真实会话。
pub(super) fn read_head_utf8(file: &Path) -> String {
    use std::io::Read;
    std::fs::File::open(file)
        .ok()
        .and_then(|mut fh| {
            let mut buf = vec![0u8; 64 * 1024];
            let n = fh.read(&mut buf).unwrap_or(0);
            buf.truncate(n);
            String::from_utf8(buf).ok()
        })
        .unwrap_or_default()
}

/// Stable `scan_progress` key for a source-log file. The same physical file
/// can surface with different path spellings across runs (e.g. a Windows home
/// dir resolving with different drive-letter case), which would otherwise fork
/// one cursor per spelling and stall the mtime gate on the stale one. Normalize
/// to lowercase + forward slashes so the key is spelling-invariant. (UTF-8
/// round-trips losslessly: source-log paths are valid Unicode in practice.)
pub(super) fn scan_progress_key(file: &Path) -> String {
    file.to_string_lossy().to_lowercase().replace('\\', "/")
}

/// Resolve the default projects dir for diagnostics (used by commands).
pub fn default_projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===================== shared normalizer =====================

    #[test]
    fn normalize_cache_inclusive_subtracts_and_clamps() {
        // Typical inclusive source: input already contains cache_read.
        let (fresh, cached) = normalize_cache_inclusive(8522, 3138);
        assert_eq!(fresh, 5384);
        assert_eq!(cached, 3138);
        // cache_read within input ⇒ both unchanged.
        let (fresh, cached) = normalize_cache_inclusive(100, 30);
        assert_eq!(fresh, 70);
        assert_eq!(cached, 30);
        // Abnormal: cache_read exceeds input (delta arithmetic) ⇒ clamped down,
        // fresh input floored at 0.
        let (fresh, cached) = normalize_cache_inclusive(10, 80);
        assert_eq!(fresh, 0);
        assert_eq!(cached, 10);
        // Both zero ⇒ both zero.
        let (fresh, cached) = normalize_cache_inclusive(0, 0);
        assert_eq!((fresh, cached), (0, 0));
    }

    // ===================== discovery skeleton invariants =====================
    //
    // The shared walker's invariants (missing root = empty not error, depth
    // cap, deterministic order, no symlink following) are pinned here once —
    // every parser's `discover` runs this exact skeleton over real tempdirs,
    // so these tests exercise the production path, not a mock.

    fn shape(root: PathBuf, max_depth: Option<u32>) -> DirectoryShape {
        DirectoryShape { root, max_depth }
    }

    #[test]
    fn discover_missing_or_empty_root_yields_empty_not_error() {
        let base = tempfile::tempdir().unwrap();
        // Missing root: an absent source dir is not an error.
        assert!(discover_files(&[shape(base.path().join("nope"), None)], is_jsonl_file).is_empty());
        // Existing but empty root: same.
        let empty = base.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(discover_files(&[shape(empty, None)], is_jsonl_file).is_empty());
    }

    #[test]
    fn discover_respects_max_depth() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        std::fs::create_dir_all(root.join("l1").join("l2")).unwrap();
        std::fs::write(root.join("a.jsonl"), "{}").unwrap(); // depth 1
        std::fs::write(root.join("l1").join("b.jsonl"), "{}").unwrap(); // depth 2
        std::fs::write(root.join("l1").join("l2").join("c.jsonl"), "{}").unwrap(); // depth 3
        fn names(files: &[PathBuf]) -> Vec<&str> {
            let mut v: Vec<&str> = files
                .iter()
                .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
                .collect();
            // readdir order is platform-dependent — compare membership only.
            v.sort_unstable();
            v
        }
        assert_eq!(
            names(&discover_files(
                &[shape(root.clone(), Some(1))],
                is_jsonl_file
            )),
            vec!["a.jsonl"]
        );
        assert_eq!(
            names(&discover_files(
                &[shape(root.clone(), Some(2))],
                is_jsonl_file
            )),
            vec!["a.jsonl", "b.jsonl"]
        );
        assert_eq!(
            names(&discover_files(&[shape(root, None)], is_jsonl_file)),
            vec!["a.jsonl", "b.jsonl", "c.jsonl"]
        );
    }

    #[test]
    fn discover_order_is_deterministic_across_calls() {
        // Same tree, two walks → byte-identical order (readdir order is stable
        // for an unchanged tree on one machine; the skeleton must not inject
        // nondeterminism such as HashMap iteration).
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        for (dir, file) in [("", "x.jsonl"), ("", "y.jsonl"), ("sub", "z.jsonl")] {
            std::fs::write(root.join(dir).join(file), "{}").unwrap();
        }
        let r1 = discover_files(&[shape(root.clone(), None)], is_jsonl_file);
        let r2 = discover_files(&[shape(root, None)], is_jsonl_file);
        assert_eq!(r1, r2, "identical order across calls");
    }

    #[test]
    fn discover_does_not_follow_symlinks() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("root");
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::fs::write(root.join("real").join("x.jsonl"), "{}").unwrap();
        // Windows needs Developer Mode (or admin) for symlinks — skip then.
        if symlink_dir(&root.join("real"), &root.join("linkdir")).is_err() {
            return;
        }
        if symlink_file(
            &root.join("real").join("x.jsonl"),
            &root.join("linkfile.jsonl"),
        )
        .is_err()
        {
            return;
        }
        let files = discover_files(&[shape(root, None)], is_jsonl_file);
        assert_eq!(
            files.len(),
            1,
            "a symlinked dir and a symlinked file are both skipped"
        );
        assert_eq!(
            files[0].file_name().and_then(|n| n.to_str()),
            Some("x.jsonl"),
            "only the real file is collected"
        );
    }

    /// `std::fs` has no portable symlink API — cfg-gated helpers for the
    /// no-symlink-following test above. Unix symlinks don't distinguish
    /// file/dir targets (`std::os::unix::fs::symlink`); the dir/file variants
    /// are Windows-only.
    #[cfg(unix)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    fn symlink_dir(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(target, link)
    }
    #[cfg(unix)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }
}
