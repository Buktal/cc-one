//! Source-log parsers (parse local session logs).
//!
//! Plugin trait + shared incremental driver. Concrete parsers live in
//! submodules (`claude`, `codex`, `gemini`, `grok`, `opencode`). Discovery
//! funnels through one shared directory-walk skeleton ([`DirectoryShape`] +
//! [`discover_files`]), each file's incremental gate is declared
//! ([`GateMode`] via [`SourceParser::gate_mode`]), and each parser's session
//! identity source is declared ([`SessionIdentity`] via
//! [`SourceParser::session_identity`]) — parsers never inline their own
//! walkers, pretend a line cursor they don't honor, or re-derive session ids
//! from a second private copy. Inside a file, the per-line fold skeleton
//! (numbering / trim / blank-skip / serde / skipped accounting) funnels
//! through the shared [`line_fold`] walker, transcript extraction through the
//! shared [`transcript`] scaffold, and the title chain through
//! [`TitleChain`], so their invariants each have a single implementation. A parser
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
mod line_fold;
mod opencode;
mod transcript;

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
    /// ISO8601 UTC timestamp from the Source log; an absent/broken source
    /// timestamp is backfilled to collection time (`fallback_timestamp`, #71).
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
    /// Entries the scan dropped without producing a row. NOT one uniform
    /// meaning across sources — the per-source declaration:
    ///   - Line-cursor JSONL sources (`claude_code`, `codex_cli`, `grok_cli`):
    ///     malformed JSON lines PAST the incremental cursor — before-cursor
    ///     lines were counted by the pass that first saw them, so a re-collect
    ///     must not recount them.
    ///   - Whole-file source (`gemini_cli` — one JSON object per file): exactly
    ///     1 when a file's entire text fails to parse (its unit is the file,
    ///     there are no lines).
    ///   - SQLite source (`opencode`): 1 when the db cannot be opened or one
    ///     of its whole-db queries fails (open / session list / session rows),
    ///     plus +1 per failed per-session usage or transcript query.
    ///   - Shared JSONL driver (all file-backed sources): +1 per discovered
    ///     file whose stat or read failed outright.
    ///
    /// Not counted anywhere: noise filtered AFTER a successful parse — unknown
    /// event types, zero-billable emit gates, codex history-replay suppression,
    /// grok's degraded half-written summary (re-read next pass) — nor codex's
    /// cheap marker substring gate, which discards non-candidate lines BEFORE
    /// any parse attempt.
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

    /// Incremental collect: parse only what each file's recorded cursor says is
    /// new, returning the advanced cursors to persist. This is the ONLY collect
    /// entry point — production (`collect_into`) calls it; with EMPTY progress
    /// it degenerates to a full scan, which is exactly what tests should use
    /// when they want "everything". 解析器自带 test-only 的
    /// `parse_full`（见各文件）满足「显式给文件列表」的旧扫面习惯。 Each parser
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
    /// files, Grok summary.json). The driver consults this per file; the
    /// default is the append-JSONL line cursor. Parsers that keep their own
    /// incremental driver (OpenCode, SQLite) never enter this one — a
    /// declaration there would be moot, so they don't override it.
    fn gate_mode(&self, _file: &Path) -> GateMode {
        GateMode::LineCursor
    }

    /// Session 身份源声明（必答，无默认）。seen 集（会话对账的「见过」集合）与
    /// fold 内每条发射行的 session_id 都从这份声明的同一条链取值（见
    /// [`SessionIdentity`]）——一致性由构造保证，不再靠人眼同步两份提取逻辑。
    /// trait 上没有「stem 默认」：对 id 不在文件名里的源（codex 线程 id /
    /// gemini sessionId / grok 会话目录名）stem 默认必然误删真实会话，危险
    /// 默认曾经只靠注释拦着；身份源是每个 parser 必须回答的声明。
    fn session_identity(&self) -> SessionIdentity;
}

/// Session 身份源（声明式数据，同 [`GateMode`] / [`DirectoryShape`] 的收法）：
/// 一个 parser 的 session id 从哪里取、取不到时沿什么链回退。seen 集
/// （reconcile 的「见过」集合）与 fold 内发射行的 id 若各写一份提取逻辑，
/// 漂移即误删——seen 报的 id 与 store 行的 id 对不上，reconcile 立刻把后者
/// 当 ghost 删掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIdentity {
    /// id = 文件 stem（claude：一会话一 jsonl，id 即 stem；discover 含
    /// `agent-*`，其行同为真实会话、staleness 由对账正常清理）。
    Stem,
    /// id = 文件的直接父目录名（grok：会话目录即会话，三个 sibling 文件同 id）。
    ParentDir,
    /// id = 单 JSON 顶层字符串字段（gemini：sessionId）；缺失/空白回退 stem。
    HeadJsonField {
        /// 顶层字段名。
        key: &'static str,
    },
    /// id = JSONL 头部首个 `session_meta` 行的线程 id（codex rollout；多键别名
    /// 宽容解析是 codex 日志格式知识，实现在 codex module），缺失回退文件名
    /// 尾随 UUID（`rollout-…-<uuid>.jsonl`），再回退 stem。
    HeadSessionMeta,
    /// 无文件身份（opencode：SQLite 会话表自管，reconcile 不适用）。seen 集
    /// 恒空 = 该 parser 的会话对账整体停用。
    SelfManaged,
}

/// 文件 stem —— claude 的身份，也是一切头读变体身份链的最终路径回退。
fn stem_or_unknown(file: &Path) -> String {
    file.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// 文件的直接父目录名 —— grok 的会话身份。
fn parent_dir_or_unknown(file: &Path) -> String {
    file.parent()
        .and_then(|dir| dir.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

impl SessionIdentity {
    /// seen 集（reconcile 的「见过」集合）——「有界读头 → 身份链」骨架的唯一
    /// 归属。对每个发现文件读 64 KiB 头（session 元数据都在文件顶部）后走
    /// [`SessionIdentity::resolve`] 同一条链；头读失败给空文本，链自然落到
    /// 路径回退——回退只会多保留一行、绝不误删真实会话。返回排序去重后的
    /// 集合：对账只论成员资格，排序去重让 collect 结果确定（discover 的
    /// walkdir 顺序平台依赖）。
    pub(super) fn seen(&self, files: &[PathBuf]) -> Vec<String> {
        if matches!(self, SessionIdentity::SelfManaged) {
            return Vec::new();
        }
        let mut ids: Vec<String> = files
            .iter()
            .map(|f| {
                let head = read_head_utf8(f);
                self.resolve(f, &head)
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// 身份链（文本入口）：头读候选（仅 codex/gemini 变体消费 `text`；路径
    /// 变体忽略，传 `""` 即可）→ 路径回退 → 终值 "unknown"。fold 的发射行与
    /// seen 集必须从同一方法（或同一变体链实现）取值。
    pub(super) fn resolve(&self, file: &Path, text: &str) -> String {
        match self {
            SessionIdentity::Stem => stem_or_unknown(file),
            SessionIdentity::ParentDir => parent_dir_or_unknown(file),
            SessionIdentity::HeadJsonField { key } => {
                let from_head = serde_json::from_str::<serde_json::Value>(text)
                    .ok()
                    .and_then(|v| v.get(key).and_then(|v| v.as_str()).map(str::to_string));
                from_head
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| stem_or_unknown(file))
            }
            SessionIdentity::HeadSessionMeta => {
                // codex rollout 的身份链实现在 codex module（线程 id 解析与
                // 文件名 UUID 都是 codex 日志格式知识）；fold 直接调同一函数，
                // 声明把「seen 与 fold 走同一条链」钉死。
                let head = codex::prescan_thread_id(text);
                codex::session_id_from(file, head.as_deref())
            }
            SessionIdentity::SelfManaged => {
                // 无文件身份：不存在按文件解析的 id。seen 恒空已在上游短路；
                // 该分支只为穷尽存在。
                String::new()
            }
        }
    }

    /// 身份链（已解析值入口）：调用方手里已有整文件 JSON（gemini fold）时免
    /// 二次解析，路径回退段与 [`SessionIdentity::resolve`] 共享。
    pub(super) fn resolve_value(&self, file: &Path, value: &serde_json::Value) -> String {
        match self {
            SessionIdentity::HeadJsonField { key } => {
                let from_head = value.get(key).and_then(|v| v.as_str()).map(str::to_string);
                from_head
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| stem_or_unknown(file))
            }
            // 其余变体没有「已解析值」的捷径，退回文本入口。
            _ => self.resolve(file, ""),
        }
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
    /// Skipped entries this file contributed — the per-source meaning is the
    /// [`CollectResult::lines_skipped`] declaration.
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
    // The declaration's `seen` sorts + dedups (walkdir order is
    // platform-dependent; a deterministic order keeps collected results stable,
    // same spirit as `order_results` above).
    let session_ids = parser.session_identity().seen(&files);
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
/// given file in full" loop (start_line 0), aggregating events / turn durations
/// / sessions / messages and applying the deterministic ordering. TEST-ONLY
/// surface (架构审查候选⑪)：生产只走 [`collect_jsonl_incremental`]——空进度即
/// 全量扫描，两者共享每文件 fold，trait 不再有自认不走生产的成员；但「给定文
/// 件列表全量扫」仍是各 parser 测试的需要，故降级为这里的 cfg(test) 驱动 +
/// 各 parser 的 `parse_full`。OpenCode（SQLite）的形状不走本驱动。
#[cfg(test)]
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
    let session_ids = parser.session_identity().seen(files);
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

/// Build the FINAL token four-pack for a cache-inclusive source: the
/// `(input, cache_read)` pair goes through [`normalize_cache_inclusive`] and
/// lands straight in a [`TokenCounts`] with no write bucket
/// (`cache_creation = 0`). The one home of the inclusive→fresh mapping —
/// Codex / Gemini / Grok all emit through this constructor, so the emitted
/// shape can never disagree with what the emit gate (`TokenCounts::is_zero`)
/// judges.
pub(super) fn fresh_token_counts(input: u32, cache_read: u32, output: u32) -> TokenCounts {
    let (fresh_input, clamped_cache_read) = normalize_cache_inclusive(input, cache_read);
    TokenCounts {
        input: fresh_input,
        output,
        cache_creation: 0,
        cache_read: clamped_cache_read,
    }
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

/// 标题链纯值类型（架构审查Ⅴ）：「首条真实 user 消息 → 截断(TITLE_MAX)」的
/// 喂入-收口原语。这条语义此前有四份手写——[`SessionMetaAcc`] 的
/// offer/finish、gemini 的内联首条 user、grok 的 summary 链、grok 的
/// `peek_first_user_message`（又一份逐行找第一条 user 消息）——只能靠人眼保持
/// 同构。两步协议：
///   - [`TitleChain::offer`]：候选 trim 后空白即拒收（「空白不算真提示」），
///     首胜——只有第一条非空候选生效；逐源噪声过滤（claude 的命令回显、
///     codex 的注入前导与 IDE 包裹）留在调用方；
///   - [`TitleChain::finish`]：extra 层（调用方自己累积的「最新者胜」层，如
///     claude 的 custom_title/summary、grok 的 generated_title）依次取首个
///     非空 → 首条 user 消息 → 调用方兜底（如 project 目录名）→ 统一
///     [`truncate`]([`TITLE_MAX`])。
///
/// 观察窗（哪些源消息算候选、兜底从哪来、何时值得为兜底层做 IO）是 per-source
/// 决策，留在调用方；本类型只收口「候选择优 + 空白判定 + 截断」这段纯值语义，
/// 故 JSONL fold 之外的源形状（gemini 单 JSON、grok 跨文件 peek）也能进来。
#[derive(Debug, Default)]
pub(super) struct TitleChain {
    /// 首条非空候选（已 trim）。
    first_user: Option<String>,
}

impl TitleChain {
    /// 喂入一个标题候选：trim 后空白即拒收；首胜，其后一切候选不再覆盖。
    pub(super) fn offer(&mut self, candidate: &str) {
        let candidate = candidate.trim();
        if self.first_user.is_none() && !candidate.is_empty() {
            self.first_user = Some(candidate.to_string());
        }
    }

    /// 首胜候选是否已定——peek 式调用方（边扫边喂、命中即停）用。
    pub(super) fn won(&self) -> bool {
        self.first_user.is_some()
    }

    /// 取走首胜候选（peek 收口用）。
    pub(super) fn take(self) -> Option<String> {
        self.first_user
    }

    /// 收口取值：extra 层依次取首个非空 → 首条 user 消息 → 调用方兜底 →
    /// 空串，统一按 [`TITLE_MAX`] 截断（[`truncate`]）。
    pub(super) fn finish(self, extra_levels: &[&str], fallback: Option<&str>) -> String {
        let title = extra_levels
            .iter()
            .copied()
            .find(|s| !s.is_empty())
            .or_else(|| self.first_user.as_deref().filter(|s| !s.is_empty()))
            .or_else(|| fallback.filter(|s| !s.is_empty()));
        truncate(title.unwrap_or(""), TITLE_MAX)
    }
}

/// Claude / Codex 共享的 session 元数据累积器（架构审查Ⅲ候选⑦）。fold 循环的
/// 另一半骨架——「session 元数据全文件重读（refreshable）、事件/消息只发游标
/// 之后」——此前在两个 parser 里各手写一份（每份约 60 行平行实现），只能靠人眼
/// 保持同构；本类型把这套步骤单源化：started_at 首置 / last_active_at 推进 /
/// 首个非空 cwd → project_dir / 首条真实用户消息作标题（经 [`TitleChain`]，
/// 噪声过滤留在调用方）/ saw_any_event 判定 / RawSession 构造。新 JSONL 源接入
/// 只声明喂入点与噪声过滤即可。
///
/// 跨 parser 不变量由此类型的使用方式表达：
///   - 三个 observe_* / offer_user_title 必须对**每一条已接受行**调用，且都写
///     在增量游标门之前——系统数据 refreshable，重采只看见尾部追加行时，首事件
///     时间/启动目录/首条提示不能丢（claude/codex 的 incremental meta 覆盖全文
///     件回归测试守此）；
///   - [`SessionMetaAcc::finish`] 在整个 fold 循环结束后调用一次（游标门之外），
///     收口 saw_any_event 语义并产出至多一个 [`RawSession`]；
///   - 游标门之后的全部发射路径不经过本类型。
///
/// 观察窗（哪些行算「已接受」）是 per-source 决策，刻意保留分叉，非遗漏：
///   - claude 对任何 serde 成功的行观察；
///   - codex 只对过 marker substring 门（`session_meta` / `turn_context` /
///     `event_msg`+`token_count` / `response_item`）的行观察——那道门是性能
///     设计（非候选行在 serde 前丢弃），并与 [`CollectResult::lines_skipped`]
///     「只计过门后的畸形行」口径绑定；放宽窗口会改动 skipped 计数语义，不属于
///     结构收敛。两窗同解的前提由日志格式保证：codex rollout 恒以 `session_meta`
///     起头、业务行全部落在四类之内。
///   - 已归一的分叉：codex 原先「过门且 serde 成功但缺 type 字段」的退化行不
///     观察元数据——现在 serde 成功即观察（type 只影响路由、不影响系统数据），
///     与 claude 的内层次序一致。
#[derive(Debug, Default)]
pub(super) struct SessionMetaAcc {
    /// 首个已接受行的原始时间戳（缺失/空串照原样冻结）；之后不再回填。
    started_at: String,
    /// 最后一次见到的非空时间戳（append-only 日志下的「最新所见」，非字典序最大）。
    last_active_at: String,
    /// 首个 trim 后非空的 cwd = 会话启动目录（CONTEXT「项目」词条：取会话启动时
    /// 的工作目录，拒绝众数钉偏 #83）。`None` = 尚未见到可用值。
    project_dir: Option<String>,
    /// 标题链纯值（首条真实用户消息层）。链 + 截断 + 空白判定的实现在
    /// [`TitleChain`]，本类型只持有并喂入；claude 的改名刷新走
    /// [`SessionExtras::extra_title_levels`] 层，与本层无关。
    title: TitleChain,
    /// 是否见过至少一行可解析事件——决定文件是否产出 session 行。
    saw_any_event: bool,
}

/// [`SessionMetaAcc::finish`] 的逐源差异面——主链之外的一切在此注入；
/// `Default` 即 codex / claude 主会话形态（空 extra 层、无固定标题、无代理标签）。
#[derive(Debug, Default)]
pub(super) struct SessionExtras<'a> {
    /// 标题链最高优先级层：依次取第一个非空值。claude 传
    /// `[custom_title, summary]`——两者都是「文件内最新者胜」（summary 会随
    /// /compact 重写、改名必须刷新标题），最新者胜的累积留在 claude 侧完成，
    /// 这里只收最终值。codex 传 state_5.sqlite 的线程标题（per-session 查找
    /// 的单值，无则传空串、链落到 user 消息层）。
    pub(super) extra_title_levels: &'a [&'a str],
    /// 固定标题（claude 子代理取 `.meta.json` 任务描述），置位时跳过整条链直接
    /// 截断——`None` 才走标准链，`Some("")` 是调用方裁决出的「强制无题」。
    pub(super) title_override: Option<&'a str>,
    pub(super) agent_type: &'a str,
    pub(super) parent_session_id: &'a str,
}

impl SessionMetaAcc {
    /// Feed one accepted line's raw timestamp. The FIRST call freezes
    /// `started_at` exactly as given — an absent/empty stamp stays "" (a file
    /// with events but no timestamps still produces a session row); every later
    /// non-empty stamp advances `last_active_at`. Also the single place where
    /// `saw_any_event` turns true.
    pub(super) fn observe_ts(&mut self, ts: Option<&str>) {
        let ts = ts.unwrap_or_default();
        if !self.saw_any_event {
            self.started_at = ts.to_string();
            self.saw_any_event = true;
        }
        if !ts.is_empty() {
            self.last_active_at = ts.to_string();
        }
    }

    /// 首个 trim 后非空的 cwd 成为 project_dir（存 trim 后的值）；其后一切——
    /// 包括 drift 进子目录的 cwd——不再覆盖。`None` / 空白串跳过。
    pub(super) fn observe_cwd(&mut self, cwd: Option<&str>) {
        if self.project_dir.is_some() {
            return;
        }
        if let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty()) {
            self.project_dir = Some(cwd.to_string());
        }
    }

    /// 标题候选择优：委托给标题链的 first-win + 空白拒收。调用方负责逐源噪声
    /// 过滤（claude 的命令回显 / codex 的注入前导与 IDE 包裹）。
    pub(super) fn offer_user_title(&mut self, candidate: &str) {
        self.title.offer(candidate);
    }

    /// 全链收口：从未观察到任何事件 → `None`（该文件的 sessions 流为空）；
    /// 否则构造 [`RawSession`]——标题链（extra 层 → 用户标题 → project 目录名
    /// 兜底 → 空串，统一截断）交 [`TitleChain::finish`]；`title_override`
    /// 置位时（claude 子代理）跳过链条只截断固定标题。
    pub(super) fn finish(
        self,
        source: &str,
        session_id: String,
        x: SessionExtras<'_>,
    ) -> Option<RawSession> {
        if !self.saw_any_event {
            return None;
        }
        let title_orig = match x.title_override {
            Some(t) => truncate(t, TITLE_MAX),
            None => {
                let basename = Path::new(self.project_dir.as_deref().unwrap_or(""))
                    .file_name()
                    .and_then(|n| n.to_str())
                    .filter(|n| !n.is_empty());
                self.title.finish(x.extra_title_levels, basename)
            }
        };
        Some(RawSession {
            id: session_id,
            source: source.to_string(),
            project_dir: self.project_dir.unwrap_or_default(),
            title_orig,
            started_at: self.started_at,
            last_active_at: self.last_active_at,
            agent_type: x.agent_type.to_string(),
            parent_session_id: x.parent_session_id.to_string(),
        })
    }
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
///
/// 适用边界（协议语义）：本策略只管要按天入账的行——[`RawUsage`] /
/// [`RawTurnDuration`] 的 timestamp，五个 parser 的用量/turn 发射全部经此
/// （含 grok 无/坏时间戳的 turn_completed：以回填时刻入账，不整条丢弃）。
/// 会话原文 [`SessionMessage`] 的 ts **不走**回填：ts 是 store 的 `(ts, uuid)`
/// 排序键，把「无时间」伪造成采集时刻会把消息整段排到对话末尾，宁缺勿假。
/// 按 source 落地：文本 JSONL 源（claude / codex / gemini / grok）原样携带、
/// 缺失即空串；opencode 的 SQLite 列是非空整型毫秒、不存在缺失形态，直接走
/// `epoch_millis_to_iso`（其历法远界外的兜底是该函数自身的排序安全策略）。
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

    /// Pin of the shared inclusive→four-pack constructor the three
    /// cache-inclusive parsers emit through. The boundary of the emit gate
    /// lives here too: with a POSITIVE inclusive input, even an abnormal
    /// `cached > input` pair clamps down to the input itself and the surviving
    /// `cache_read` (and any output) keeps the row billable; an all-zero pack
    /// — including cache claimed against a ZERO input with nothing else — is
    /// what gets dropped.
    #[test]
    fn fresh_token_counts_maps_and_bounds_the_emit_gate() {
        // Typical inclusive pair: input de-cached, buckets land as expected.
        let t = fresh_token_counts(8522, 3138, 29);
        assert_eq!(
            (t.input, t.cache_read, t.output, t.cache_creation),
            (5384, 3138, 29, 0)
        );
        // Cache fully covers input (an all-cache turn): still billable —
        // cache_read survives.
        let t = fresh_token_counts(5000, 5000, 0);
        assert_eq!((t.input, t.cache_read, t.output), (0, 5000, 0));
        assert!(!t.is_zero(), "well-formed cache-only row is billable");
        // Abnormal cached > input, positive input: cached clamps DOWN TO the
        // input, so the row carries exactly that much billable cache (plus
        // whatever output) — never dropped on the clamp alone.
        let t = fresh_token_counts(10, 80, 60);
        assert_eq!((t.input, t.cache_read, t.output), (0, 10, 60));
        assert!(!t.is_zero(), "output saves an abnormal cache-only row");
        let t = fresh_token_counts(5, 9, 0);
        assert_eq!((t.input, t.cache_read, t.output), (0, 5, 0));
        assert!(
            !t.is_zero(),
            "clamped cache_read == input alone keeps the row alive"
        );
        // Cache claimed against a ZERO inclusive input, nothing else: no
        // bucket survives ⇒ dropped by the gate.
        let t = fresh_token_counts(0, 9, 0);
        assert_eq!((t.input, t.cache_read, t.output), (0, 0, 0));
        assert!(t.is_zero(), "cache against zero input ⇒ dropped");
        assert!(
            fresh_token_counts(0, 0, 0).is_zero(),
            "all-zero source row ⇒ dropped"
        );
    }

    // ===================== SessionMetaAcc =====================

    #[test]
    fn session_meta_acc_finishes_to_nothing_until_an_event_is_observed() {
        let fresh = SessionMetaAcc::default();
        assert!(
            fresh
                .finish("claude_code", "s".into(), SessionExtras::default())
                .is_none(),
            "a file with no accepted event yields no session"
        );
        let mut observed = SessionMetaAcc::default();
        observed.observe_ts(None);
        assert!(
            observed
                .finish("claude_code", "s".into(), SessionExtras::default())
                .is_some(),
            "one observed line is enough"
        );
    }

    /// Time boundaries: the first observed line freezes started_at exactly as
    /// seen (absent → "", not backfilled by later stamps); last_active_at
    /// follows every later NON-empty stamp — latest seen, NOT lexical max
    /// (append-only logs arrive in order; an out-of-order earlier stamp must
    /// still overwrite, which is what both parsers always did).
    #[test]
    fn session_meta_acc_freezes_started_at_and_advances_last_active() {
        let mut acc = SessionMetaAcc::default();
        // First line without a timestamp: started_at freezes "" but the
        // session row still exists.
        acc.observe_ts(None);
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.observe_ts(Some(""));
        acc.observe_ts(Some("2026-08-02T09:00:00Z"));
        let s = acc
            .finish("claude_code", "sid".into(), SessionExtras::default())
            .unwrap();
        assert_eq!(s.started_at, "");
        assert_eq!(s.last_active_at, "2026-08-02T09:00:00Z");
        assert_eq!(s.id, "sid");
        assert_eq!(s.source, "claude_code");
        assert_eq!(s.agent_type, "");
        assert_eq!(s.parent_session_id, "");

        // A normal first line starts AND anchors last_active in one call.
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        let s = acc
            .finish("codex_cli", "sid".into(), SessionExtras::default())
            .unwrap();
        assert_eq!(s.started_at, "2026-08-01T10:00:00Z");
        assert_eq!(s.last_active_at, "2026-08-01T10:00:00Z");
    }

    /// project_dir = first TRIMMED non-empty cwd; nothing after it overwrites;
    /// no usable cwd anywhere degrades to "" (and hence no basename fallback).
    #[test]
    fn session_meta_acc_project_dir_is_first_non_blank_trimmed_cwd() {
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.observe_cwd(None);
        acc.observe_cwd(Some("   "));
        acc.observe_cwd(Some(" /home/me/proj "));
        // Later cwds — even more of them, deeper — never replace the launch dir.
        acc.observe_cwd(Some("/home/me/proj/sub"));
        let s = acc
            .finish("claude_code", "id".into(), SessionExtras::default())
            .unwrap();
        assert_eq!(s.project_dir, "/home/me/proj");

        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        let s = acc
            .finish("claude_code", "id".into(), SessionExtras::default())
            .unwrap();
        assert_eq!(s.project_dir, "");
        assert_eq!(s.title_orig, "", "no cwd ⇒ no basename ⇒ empty title");
    }

    /// Title chain precedence: extra levels beat the first-win user title,
    /// which beats the project-dir basename. Blank offers are rejected (the
    /// callers pre-filter noise), and a won offer is final.
    #[test]
    fn session_meta_acc_title_chain_extras_then_user_then_basename() {
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.observe_cwd(Some("/home/me/O_cc one"));
        acc.offer_user_title("   ");
        acc.offer_user_title("First real prompt");
        acc.offer_user_title("Second prompt");
        // An empty extra level is skipped; the non-empty one wins everything.
        let s = acc
            .finish(
                "claude_code",
                "id".into(),
                SessionExtras {
                    extra_title_levels: &["", "Renamed"],
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(s.title_orig, "Renamed");

        // Without extras the first-win user title leads.
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.observe_cwd(Some("/home/me/O_cc one"));
        acc.offer_user_title("First real prompt");
        let s = acc
            .finish("codex_cli", "id".into(), SessionExtras::default())
            .unwrap();
        assert_eq!(s.title_orig, "First real prompt");

        // No title source at all → basename of the launch dir.
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.observe_cwd(Some("/home/me/O_cc one"));
        let s = acc
            .finish("codex_cli", "id".into(), SessionExtras::default())
            .unwrap();
        assert_eq!(s.title_orig, "O_cc one");
    }

    /// The chain result AND a fixed title are truncated identically at
    /// TITLE_MAX with the shared ellipsis rule.
    #[test]
    fn session_meta_acc_truncates_chain_and_override_titles() {
        let long = "x".repeat(200);
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.offer_user_title(&long);
        let s = acc
            .finish("codex_cli", "id".into(), SessionExtras::default())
            .unwrap();
        assert!(s.title_orig.ends_with('…'));
        assert!(s.title_orig.chars().count() <= TITLE_MAX);

        // A title_override is truncated too; agent_type/parent ride along.
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        acc.offer_user_title("ignored while overridden");
        let s = acc
            .finish(
                "claude_code",
                "agent-x".into(),
                SessionExtras {
                    title_override: Some(&long),
                    agent_type: "Explore",
                    parent_session_id: "p1",
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(s.title_orig.chars().count() <= TITLE_MAX);
        assert_eq!(s.agent_type, "Explore");
        assert_eq!(s.parent_session_id, "p1");

        // An override within bounds passes through untouched.
        let mut acc = SessionMetaAcc::default();
        acc.observe_ts(Some("2026-08-01T10:00:00Z"));
        let s = acc
            .finish(
                "claude_code",
                "agent-y".into(),
                SessionExtras {
                    title_override: Some("核实 cc-switch 供应商"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(s.title_orig, "核实 cc-switch 供应商");
    }

    // ===================== SessionIdentity（身份源声明） =====================

    /// 各变体的身份链（resolve）：Stem / ParentDir 是纯路径取值；HeadJsonField
    /// 头字段命中即取，缺失 / 空串 / 非法 JSON 都回退 stem——回退只会多保留
    /// 一行、绝不误删真实会话；路径解析不出 stem 时终值 "unknown"。
    #[test]
    fn session_identity_resolve_chains() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("proj").join("sess-file.jsonl");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();

        assert_eq!(SessionIdentity::Stem.resolve(&file, ""), "sess-file");
        assert_eq!(
            SessionIdentity::ParentDir.resolve(&file, ""),
            "proj",
            "grok: id = 直接父目录名"
        );
        let gem = SessionIdentity::HeadJsonField { key: "sessionId" };
        assert_eq!(gem.resolve(&file, r#"{"sessionId":"real-id"}"#), "real-id");
        assert_eq!(gem.resolve(&file, "{}"), "sess-file", "字段缺失 → stem");
        assert_eq!(
            gem.resolve(&file, r#"{"sessionId":""}"#),
            "sess-file",
            "空串不算身份"
        );
        assert_eq!(
            gem.resolve(&file, "{not json"),
            "sess-file",
            "非法 JSON → stem"
        );
        assert_eq!(
            SessionIdentity::Stem.resolve(Path::new("/"), ""),
            "unknown",
            "解析不出 stem 的终值"
        );
    }

    /// resolve_value（已解析值入口，gemini fold 用）与 resolve（文本入口，
    /// seen 集用）同链同果——「fold 与 seen 不漂移」在两个入口上同时钉住。
    #[test]
    fn session_identity_value_entry_matches_text_entry() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sess-x.json");
        let gem = SessionIdentity::HeadJsonField { key: "sessionId" };
        let hit: serde_json::Value = serde_json::from_str(r#"{"sessionId":"abc"}"#).unwrap();
        assert_eq!(
            gem.resolve_value(&file, &hit),
            gem.resolve(&file, r#"{"sessionId":"abc"}"#)
        );
        let miss: serde_json::Value = serde_json::from_str("{}").unwrap();
        assert_eq!(gem.resolve_value(&file, &miss), "sess-x");
    }

    /// seen 集：SelfManaged 恒空（opencode 的对账停用）；头读失败（文件不在）
    /// 沿链落到路径回退，照常产出 id；多文件同 id（grok 三 sibling）排序去重。
    #[test]
    fn session_identity_seen_dedups_sorts_and_selfmanaged_is_empty() {
        assert!(SessionIdentity::SelfManaged
            .seen(&[PathBuf::from("/x/summary.json")])
            .is_empty());
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("s1").join("summary.json");
        let b = dir.path().join("s1").join("chat_history.jsonl");
        let c = dir.path().join("s2").join("chat_history.jsonl");
        assert_eq!(
            SessionIdentity::ParentDir.seen(&[a, b, c]),
            vec!["s1".to_string(), "s2".to_string()],
            "三 sibling 同 id 收成一条，且有序"
        );
    }

    // ===================== TitleChain（标题链纯值） =====================

    /// offer 的「空白不算真提示 + 首胜」：空白候选拒收，首条非空候选生效后
    /// 其后一切不再覆盖（peek 式调用方靠 won/take 命中即停）。
    #[test]
    fn title_chain_offer_blank_reject_and_first_win() {
        let mut t = TitleChain::default();
        t.offer("   ");
        assert!(!t.won(), "空白不是候选");
        t.offer("First real prompt");
        assert!(t.won());
        t.offer("Second prompt");
        assert_eq!(t.take().as_deref(), Some("First real prompt"));
    }

    /// finish 的层次：extra 层依次取首个非空 → user 层 → 调用方兜底 → 空串，
    /// 统一 TITLE_MAX 截断。gemini 形状（无 extra 无兜底）与 grok 形状
    /// （双层 extra 无兜底）都钉在这里。
    #[test]
    fn title_chain_finish_precedence_extra_then_user_then_fallback() {
        let mut t = TitleChain::default();
        t.offer("from user");
        assert_eq!(t.finish(&["", "extra"], Some("basename")), "extra");
        let mut t = TitleChain::default();
        t.offer("from user");
        assert_eq!(t.finish(&[], Some("basename")), "from user");
        let t = TitleChain::default();
        assert_eq!(t.finish(&[], Some("basename")), "basename");
        assert_eq!(TitleChain::default().finish(&[], None), "");

        // gemini 形状：offer 值 trim 后入链，finish 统一截断。
        let mut t = TitleChain::default();
        t.offer("  hi  ");
        assert_eq!(t.finish(&[], None), "hi");
        let long = "x".repeat(200);
        let mut t = TitleChain::default();
        t.offer(&long);
        let out = t.finish(&[], None);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= TITLE_MAX);
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
