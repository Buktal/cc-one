//! Gemini CLI (`~/.gemini`) session-log parser.

use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::model::{RawSession, SessionMessage, SessionMessageRole};

use super::transcript::{MessageSpec, MessageUuid, RawMessage, TextKeys};
use super::{
    collect_jsonl_incremental, discover_files, fresh_token_counts, CollectResult, DirectoryShape,
    FileParseOutcome, GateMode, RawUsage, ScanProgress, ScanProgressDelta, SessionIdentity,
    SourceParser, TitleChain,
};

/// Stable source tag — becomes `RawUsage.source` / `RawSession.source` and the
/// DB source column; the single literal behind `name()`, usage, and session
/// construction.
const SOURCE_TAG: &str = "gemini_cli";

/// Gemini 的会话身份源声明（单 JSON 顶层的 `sessionId`；缺失/空白回退文件
/// stem）——trait 声明与 fold 取值共用这一份（fold 手里已有整文件 JSON，走
/// `resolve_value` 免二次解析；链与 seen 集同一条）。
const SESSION_IDENTITY: SessionIdentity = SessionIdentity::HeadJsonField { key: "sessionId" };

/// Gemini 的 transcript 提取声明（content key 表 + role 词典 + uuid 规则）——
/// transcript 提取与标题候选路径共用同一份；骨架与统一空文本决策见
/// [`super::transcript`]。
static MESSAGES: MessageSpec = MessageSpec {
    text_keys: TextKeys {
        block_type: None,
        keys: &["text"],
    },
    roles: &[
        ("user", SessionMessageRole::User),
        ("gemini", SessionMessageRole::Assistant),
    ],
    uuid_rule: MessageUuid::RequiredSourceId,
};

/// Gemini CLI (`~/.gemini`) session-log parser.
///
/// Reads `<gemini_dir>/tmp/<project_hash>/chats/session-*.json`. Each file is a
/// single JSON object with a `messages` array; only `type:"gemini"` messages
/// carrying a `tokens` object are consumed. The CLI's `input` is
/// cache-inclusive (it contains `cached`), so it is normalized to fresh at
/// parse; `cached` is cache_read, and `thoughts` is folded into `output`
/// (thinking tokens are billed as output). `cache_creation` is always 0 —
/// Gemini uses implicit caching and does not expose a write bucket.
pub struct GeminiCliSourceParser {
    gemini_dir: PathBuf,
}

impl GeminiCliSourceParser {
    /// Root-injection seam: parser rooted at `home/.gemini`. The collect
    /// orchestration factory (`all_source_parsers_at`) builds every parser
    /// through this seam, so tests can point the whole chain at a tempdir
    /// fixture instead of the real `~`.
    pub(crate) fn new_at(home: &Path) -> Self {
        Self {
            gemini_dir: home.join(".gemini"),
        }
    }

    /// Test/override constructor with an explicit gemini dir.
    #[cfg(test)]
    pub(crate) fn with_dir(dir: PathBuf) -> Self {
        Self { gemini_dir: dir }
    }
}

impl SourceParser for GeminiCliSourceParser {
    fn name(&self) -> &'static str {
        SOURCE_TAG
    }

    fn discover(&self) -> AppResult<Vec<PathBuf>> {
        // Fixed two-level shape `tmp/<project_hash>/chats/session-*.json`. A
        // missing gemini dir (no CLI install / no sessions yet) is not an
        // error — the shared skeleton yields no files for an absent root.
        Ok(discover_files(
            &[DirectoryShape {
                root: self.gemini_dir.join("tmp"),
                max_depth: Some(3), // tmp/<hash>/chats/*.json
            }],
            is_gemini_session_file,
        ))
    }

    /// Gemini session ids live in the file's JSON `sessionId`, not the stem —
    /// reconcile needs the real ids or it would mis-delete sessions. Declared
    /// as [`SessionIdentity::HeadJsonField`]; the shared seen skeleton does the
    /// bounded head read and falls back to the stem on any failure (a fallback
    /// can only KEEP an extra row, never delete a real session).
    fn session_identity(&self) -> SessionIdentity {
        SESSION_IDENTITY
    }

    /// Incremental collect: a Gemini session file is a single JSON object, so
    /// there is no line cursor — [`GateMode::MtimeOnly`] declares that, and
    /// the shared driver mtime-gates the file and re-parses it in full
    /// (recording no line offset). The store dedups already-seen message ids
    /// at ingest; a CLI rewrite that changes an existing message's tokens is
    /// NOT re-costed (freeze + top-up only), which matches the session-log
    /// contract.
    fn collect_incremental(
        &self,
        progress: &ScanProgress,
    ) -> AppResult<(CollectResult, ScanProgressDelta)> {
        collect_jsonl_incremental(self, progress, |file, text, _start_line| {
            fold_file(file, text)
        })
    }

    /// A Gemini session file is one JSON object — mtime-only, no line cursor.
    /// Declared (not pretended) so the shared driver records no line offset
    /// for these files.
    fn gate_mode(&self, _file: &Path) -> GateMode {
        GateMode::MtimeOnly
    }
}

/// Gemini session files live at `tmp/<project_hash>/chats/session-*.json` —
/// the `chats` parent check pins the shape so sibling files elsewhere (e.g.
/// `.project_root`) can never match.
fn is_gemini_session_file(path: &Path) -> bool {
    let in_chats = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("chats");
    in_chats
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("session-") && n.ends_with(".json"))
}

/// Parsed token fields from a Gemini `tokens` object (pre-thoughts-merge).
/// Pure field extraction — no zero/billable judgment happens on this
/// intermediate shape; that gate lives once on the final [`TokenCounts`].
struct GeminiTokens {
    input: u32,
    output: u32,
    cached: u32,
    thoughts: u32,
}

fn parse_gemini_tokens(tokens: &serde_json::Value) -> GeminiTokens {
    let n = |k: &str| tokens.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    GeminiTokens {
        input: n("input"),
        output: n("output"),
        cached: n("cached"),
        thoughts: n("thoughts"),
    }
}

/// Fold one Gemini session file's text into a per-file parse outcome. Three
/// streams from a single forward pass over the JSON `messages` array:
///   - per-call usages (`type:"gemini"` with a usable `tokens` object);
///   - one [`RawSession`] (system data: id / source / project_dir / title /
///     timestamps) — meta is rebuilt every pass so a re-collect refreshes it;
///   - transcript [`SessionMessage`]s (`type:"user"`→User, `type:"gemini"`→
///     Assistant; `info`/`error`/unknown skipped).
///
/// `session_id` (the JSON `sessionId`) is stamped onto every emitted
/// [`RawUsage`] and [`SessionMessage`]. Gemini rewrites the whole JSON per edit
/// (not append), so there is no line cursor — the shared driver's mtime gate
/// triggers a full re-parse, and the ledger dedups already-seen message ids.
fn fold_file(file: &Path, text: &str) -> FileParseOutcome {
    // The file IS the unit here (one JSON object, not append-only JSONL): a
    // failed whole-file parse counts as exactly ONE skipped entry — see the
    // `lines_skipped` declaration in `super`.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return FileParseOutcome {
            events: Vec::new(),
            corrections: Vec::new(),
            turn_durations: Vec::new(),
            sessions: Vec::new(),
            messages: Vec::new(),
            skipped: 1,
        };
    };
    // 会话身份走声明（sessionId → stem 回退）——与 trait 声明、seen 集同一条
    // 链取值；整文件 JSON 已在手，走 `resolve_value` 免二次解析。（旧 fold 在
    // sessionId 缺失时填 "unknown"，与 seen 集的 stem 回退分叉——那条分叉会让
    // 对账把该会话行当 ghost 误删，收口后两处必然一致。）
    let session_id = SESSION_IDENTITY.resolve_value(file, &value);
    let project_dir = read_project_root(file);
    let started_at = value
        .get("startTime")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_active_at = value
        .get("lastUpdated")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let mut events = Vec::new();
    let mut messages: Vec<SessionMessage> = Vec::new();
    // 标题链纯值：gemini 只有「首条真实 user 消息」一层（无 summary 层、无
    // basename 兜底——project_dir 来自旁置 .project_root 文件，不作标题）。
    let mut title = TitleChain::default();

    if let Some(msgs) = value.get("messages").and_then(|v| v.as_array()) {
        for msg in msgs {
            let typ = msg.get("type").and_then(|t| t.as_str());

            // Original title candidate: first user message's content (string
            // or first text block). 首胜与空白判定归标题链。
            if typ == Some("user") {
                if let Some(t) = MESSAGES.text_keys.first_text(msg.get("content")) {
                    title.offer(&t);
                }
            }

            // Per-call usage (type:"gemini" with a usable tokens object).
            if typ == Some("gemini") {
                if let Some(u) = extract_usage(msg, &session_id) {
                    events.push(u);
                }
            }

            // Transcript message — the shared scaffold: role dictionary drops
            // info/error/unknown, RequiredSourceId skips id-less lines, and the
            // empty-content policy lives in the declaration.
            if let Some(m) = MESSAGES.emit(RawMessage {
                session_id: &session_id,
                role_str: typ.unwrap_or(""),
                content: msg.get("content"),
                ts: msg.get("timestamp").and_then(|v| v.as_str()).unwrap_or(""),
                model: if typ == Some("gemini") {
                    msg.get("model")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                } else {
                    None
                },
                name: first_tool_name(msg),
                source_id: msg.get("id").and_then(|v| v.as_str()),
                line_no: 0,
                uuid_suffix: "",
            }) {
                messages.push(m);
            }
        }
    }

    FileParseOutcome {
        events,
        corrections: Vec::new(),
        turn_durations: Vec::new(),
        sessions: vec![RawSession {
            id: session_id,
            source: SOURCE_TAG.to_string(),
            project_dir,
            title_orig: title.finish(&[], None),
            started_at,
            last_active_at,
            agent_type: String::new(),
            parent_session_id: String::new(),
        }],
        messages,
        skipped: 0,
    }
}

/// Extract a per-call [`RawUsage`] from a `type:"gemini"` message carrying a
/// usable `tokens` object. Returns `None` for missing/non-object tokens, or
/// when the FINAL four-pack is all zero — a fully-zero `tokens` object, or one
/// that claims cache reads against a ZERO inclusive input with no output or
/// thoughts anywhere. A positive input keeps its clamped cache ride billable.
/// `session_id` is stamped onto the result so usage rows carry their session
/// grouping key.
fn extract_usage(msg: &serde_json::Value, session_id: &str) -> Option<RawUsage> {
    let tokens_obj = msg.get("tokens")?;
    if !tokens_obj.is_object() {
        return None;
    }
    let parsed = parse_gemini_tokens(tokens_obj);
    // Gemini's `input` is cache-inclusive (it already contains `cached`);
    // thoughts are billed as output. Map to the FINAL four-pack first, then
    // judge it once through the shared emit gate (`TokenCounts::is_zero`).
    let tokens = fresh_token_counts(parsed.input, parsed.cached, parsed.output + parsed.thoughts);
    if tokens.is_zero() {
        return None;
    }
    let message_id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
    let model = msg
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let timestamp = msg
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(RawUsage {
        uuid: format!("gemini:{session_id}:{message_id}"),
        timestamp: super::fallback_timestamp(timestamp),
        model: model.to_string(),
        source: SOURCE_TAG.to_string(),
        session_id: session_id.to_string(),
        tokens,
        ..Default::default()
    })
}

/// Read the sibling `.project_root` file (at `tmp/<hash>/.project_root`, two
/// levels up from the session file) for the session's working directory.
/// Returns an empty string when the file is absent or unreadable — Gemini does
/// not record `cwd` inside the session JSON.
fn read_project_root(file: &Path) -> String {
    let Some(hash_dir) = file.parent().and_then(|p| p.parent()) else {
        return String::new();
    };
    std::fs::read_to_string(hash_dir.join(".project_root"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

/// First tool-call name from a message's optional `toolCalls` array. Used to
/// populate the `name` field on assistant transcript messages.
fn first_tool_name(msg: &serde_json::Value) -> Option<String> {
    msg.get("toolCalls")
        .and_then(|v| v.as_array())
        .and_then(|calls| {
            calls.iter().find_map(|c| {
                c.get("name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
        })
}

// ---------------------------------------------------------- 测试全量扫面 --
// 昔年 trait 成员 `parse` 的降级（架构审查候选⑪）：test-only、走生产同款驱动
// （|file, text, _| fold_file(file, text) 等 fold 与 `collect_incremental` 共用），但不再是谎称生产的
// trait 接口；需要「显式文件列表全量扫」的测试改走 parse_full。
#[cfg(test)]
impl GeminiCliSourceParser {
    pub(crate) fn parse_full(&self, files: &[PathBuf]) -> AppResult<CollectResult> {
        super::parse_jsonl_full(self, files, |file, text, _| fold_file(file, text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SessionMessage, SessionMessageRole};
    use crate::source_parser::TRIM_LIMIT;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    fn write_gemini_session(dir: &Path, hash: &str, filename: &str, json: &str) -> PathBuf {
        let path = dir.join("tmp").join(hash).join("chats").join(filename);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json).unwrap();
        path
    }

    /// Write the sibling `.project_root` (one line = the project absolute path)
    /// at `tmp/<hash>/.project_root` — two levels above the session file.
    fn write_project_root(dir: &Path, hash: &str, project_path: &str) {
        let path = dir.join("tmp").join(hash).join(".project_root");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, project_path).unwrap();
    }

    #[test]
    fn gemini_parse_tokens_variants() {
        let full: serde_json::Value = serde_json::json!({
            "input": 8522, "output": 29, "cached": 3138, "thoughts": 405, "tool": 0, "total": 8956
        });
        let t = parse_gemini_tokens(&full);
        assert_eq!(t.input, 8522);
        assert_eq!(t.output, 29);
        assert_eq!(t.cached, 3138);
        assert_eq!(t.thoughts, 405);
        // missing fields ⇒ 0.
        let partial: serde_json::Value = serde_json::json!({ "input": 100, "output": 50 });
        let t = parse_gemini_tokens(&partial);
        assert_eq!(t.cached, 0);
        assert_eq!(t.thoughts, 0);
    }

    #[test]
    fn gemini_discover_missing_dir_returns_empty() {
        let base = tempfile::tempdir().unwrap();
        let p = GeminiCliSourceParser::with_dir(base.path().join("nope"));
        assert!(p.discover().unwrap().is_empty());
    }

    /// Four-bucket mapping: Gemini's `input` is cache-inclusive in the source —
    /// normalized to fresh (input − cache_read, clamped) at parse — output folds
    /// in thoughts, cache_creation = 0. The fixture's `total` (8522 + 29 + 405
    /// = 8956) must round-trip to parsed.total() once input is de-cached.
    /// All-zero messages are dropped; a genuine cache-only row (cached ≤ its
    /// inclusive input) is billable and KEPT.
    #[test]
    fn gemini_parses_session_into_fresh_four_buckets() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{
            "sessionId": "s1",
            "messages": [
                {"type":"gemini","id":"m1","model":"gemini-2.5-pro","timestamp":"2026-07-15T12:34:56.789Z","tokens":{"input":8522,"output":29,"cached":3138,"thoughts":405,"tool":0,"total":8956}},
                {"type":"gemini","id":"m2","model":"gemini-2.5-pro","timestamp":"2026-07-15T12:35:00.000Z","tokens":{"input":5000,"output":0,"cached":5000,"thoughts":0}},
                {"type":"gemini","id":"m3","model":"gemini-2.5-pro","timestamp":"2026-07-15T12:35:01.000Z","tokens":{"input":0,"output":0,"cached":0,"thoughts":0}},
                {"type":"user","id":"u1","message":"hi"}
            ]
        }"#;
        write_gemini_session(dir.path(), "hashA", "session-1.json", json);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.source, "gemini_cli");
        assert_eq!(
            result.events.len(),
            2,
            "m3 all-zero + user dropped; m1/m2 kept"
        );
        let by_id: std::collections::HashMap<&str, &RawUsage> =
            result.events.iter().map(|e| (e.uuid.as_str(), e)).collect();
        let m1 = by_id["gemini:s1:m1"];
        // Fresh input = inclusive input (8522) − cache_read (3138).
        assert_eq!(m1.tokens.input, 5384, "input de-cached: 8522 - 3138");
        assert_eq!(m1.tokens.output, 434, "output folds in thoughts (29 + 405)");
        assert_eq!(m1.tokens.cache_read, 3138);
        assert_eq!(m1.tokens.cache_creation, 0);
        assert_eq!(m1.model, "gemini-2.5-pro");
        // Invariant: de-caching preserves the fixture total (fresh input +
        // output + cache_read == inclusive input + output + thoughts).
        assert_eq!(
            m1.tokens.total(),
            8956,
            "fresh input + output + cache_read == fixture total"
        );
        let m2 = by_id["gemini:s1:m2"];
        assert_eq!(m2.tokens.cache_read, 5000, "cache-only message kept");
        assert_eq!(m2.tokens.input, 0);
        assert_eq!(m2.tokens.output, 0);
    }

    #[test]
    fn gemini_incremental_mtime_gates_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"sessionId":"s1","messages":[{"type":"gemini","id":"m1","model":"gemini-2.5-pro","timestamp":"2026-07-15T12:34:56.789Z","tokens":{"input":10,"output":1,"cached":2,"thoughts":0}}]}"#;
        let path = write_gemini_session(dir.path(), "h", "session-1.json", json);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let (r1, delta) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.events.len(), 1);
        // mtime-only gate: the recorded cursor carries NO line offset — the
        // fake line-cursor contract must not leak here.
        let (_, cursor) = delta.iter().next().expect("cursor recorded");
        assert_eq!(
            cursor.last_line_offset, 0,
            "no line cursor for Gemini files"
        );
        let progress: ScanProgress = delta;
        // Unchanged file ⇒ mtime gate skips it entirely.
        let (r2, delta2) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r2.events.len(), 0);
        assert!(delta2.is_empty());
        // Rewrite (new mtime) ⇒ full re-parse; the seen id is re-emitted (the
        // store dedups at ingest, not here).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, json).unwrap();
        let (r3, _) = p.collect_incremental(&progress).unwrap();
        assert_eq!(r3.events.len(), 1);
    }

    // ---- session management: RawSession + SessionMessage + session_id ----

    /// Full session extraction: RawSession (project_dir from sibling
    /// .project_root, title from first user content, timestamps from
    /// startTime/lastUpdated), SessionMessage (role mapping including
    /// gemini→Assistant, content join, tool name, uuid = message id), and
    /// RawUsage.session_id stamped. info/error messages are skipped.
    #[test]
    fn gemini_parses_session_meta_messages_and_usage_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{
            "sessionId": "sess-abc",
            "startTime": "2026-08-01T10:00:00.000Z",
            "lastUpdated": "2026-08-01T11:30:00.000Z",
            "messages": [
                {"id":"u1","type":"user","timestamp":"2026-08-01T10:00:00.000Z","content":"Hello Gemini"},
                {"id":"g1","type":"gemini","timestamp":"2026-08-01T10:01:00.000Z","model":"gemini-2.5-pro","content":[{"text":"Hi"},{"text":"How can I help?"}],"toolCalls":[{"name":"read_file","args":{}}],"tokens":{"input":100,"output":50,"cached":10,"thoughts":5}},
                {"id":"i1","type":"info","timestamp":"2026-08-01T10:02:00.000Z","content":"system info"},
                {"id":"e1","type":"error","timestamp":"2026-08-01T10:03:00.000Z","content":"oops"},
                {"id":"u2","type":"user","timestamp":"2026-08-01T11:00:00.000Z","content":"Thanks"}
            ]
        }"#;
        write_gemini_session(dir.path(), "hashA", "session-abc.json", json);
        write_project_root(dir.path(), "hashA", "/home/me/project");

        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();

        // ---- RawSession ----
        assert_eq!(result.sessions.len(), 1);
        let s = &result.sessions[0];
        assert_eq!(s.id, "sess-abc");
        assert_eq!(s.source, "gemini_cli");
        assert_eq!(s.project_dir, "/home/me/project"); // from .project_root
        assert_eq!(s.title_orig, "Hello Gemini"); // first user content
        assert_eq!(s.started_at, "2026-08-01T10:00:00.000Z");
        assert_eq!(s.last_active_at, "2026-08-01T11:30:00.000Z");

        // ---- SessionMessages: u1 + g1 + u2 kept; i1 + e1 skipped ----
        assert_eq!(result.messages.len(), 3);
        let by_id: HashMap<&str, &SessionMessage> = result
            .messages
            .iter()
            .map(|m| (m.uuid.as_str(), m))
            .collect();

        let u1 = by_id["u1"];
        assert_eq!(u1.role, SessionMessageRole::User);
        assert_eq!(u1.content, "Hello Gemini");
        assert_eq!(u1.session_id, "sess-abc");
        assert_eq!(u1.ts, "2026-08-01T10:00:00.000Z");
        assert!(u1.model.is_none(), "user messages have no model");
        assert!(u1.name.is_none());

        // gemini → Assistant; content blocks joined with \n; tool name captured.
        let g1 = by_id["g1"];
        assert_eq!(g1.role, SessionMessageRole::Assistant);
        assert_eq!(g1.content, "Hi\nHow can I help?");
        assert_eq!(g1.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(g1.name.as_deref(), Some("read_file"));
        assert_eq!(g1.session_id, "sess-abc");

        let u2 = by_id["u2"];
        assert_eq!(u2.role, SessionMessageRole::User);
        assert_eq!(u2.content, "Thanks");

        // info / error are NOT in the transcript.
        assert!(!by_id.contains_key("i1"), "info skipped");
        assert!(!by_id.contains_key("e1"), "error skipped");

        // ---- RawUsage.session_id filled ----
        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].session_id, "sess-abc");
        assert_eq!(result.events[0].uuid, "gemini:sess-abc:g1");
    }

    /// Message uuids are the source message ids — stable across re-parses so
    /// the ingest ledger can dedup idempotently.
    #[test]
    fn gemini_message_uuids_stable_across_reparses() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"sessionId":"s1","messages":[
            {"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"first"},
            {"id":"m2","type":"gemini","timestamp":"2026-08-01T10:01:00Z","model":"gemini-2.5-pro","content":"reply"}
        ]}"#;
        write_gemini_session(dir.path(), "h", "session-1.json", json);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());

        let r1 = p.parse_full(&p.discover().unwrap()).unwrap();
        let r2 = p.parse_full(&p.discover().unwrap()).unwrap();
        let ids1: Vec<&str> = r1.messages.iter().map(|m| m.uuid.as_str()).collect();
        let ids2: Vec<&str> = r2.messages.iter().map(|m| m.uuid.as_str()).collect();
        assert_eq!(ids1, vec!["m1", "m2"]);
        assert_eq!(ids1, ids2, "uuids stable across re-parses");
    }

    /// Missing sibling `.project_root` ⇒ project_dir is an empty string (the
    /// session JSON does not carry `cwd`).
    #[test]
    fn gemini_project_dir_empty_without_project_root_file() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"sessionId":"s1","startTime":"2026-08-01T10:00:00Z","lastUpdated":"2026-08-01T10:00:00Z","messages":[{"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"hi"}]}"#;
        write_gemini_session(dir.path(), "h", "session-1.json", json);
        // No .project_root written.
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.sessions[0].project_dir, "");
    }

    /// Content over TRIM_LIMIT (32 KiB) is truncated with an ellipsis.
    #[test]
    fn gemini_message_content_truncated_over_trim_limit() {
        let dir = tempfile::tempdir().unwrap();
        let long_text = "x".repeat(TRIM_LIMIT + 1000);
        let json = format!(
            r#"{{"sessionId":"s1","messages":[{{"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"{long_text}"}}]}}"#
        );
        write_gemini_session(dir.path(), "h", "session-1.json", &json);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        assert_eq!(result.messages.len(), 1);
        let content = &result.messages[0].content;
        assert!(content.ends_with('…'), "truncated with ellipsis");
        assert!(content.chars().count() <= TRIM_LIMIT);
    }

    /// Incremental: a rewrite (new mtime) triggers a full re-parse — both meta
    /// and messages are rebuilt. The parser re-emits already-seen message ids
    /// (the ledger dedups at ingest, not here).
    #[test]
    fn gemini_incremental_rebuilds_meta_and_messages_on_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let json_v1 = r#"{"sessionId":"s1","startTime":"2026-08-01T10:00:00Z","lastUpdated":"2026-08-01T10:00:00Z","messages":[{"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"first"}]}"#;
        let path = write_gemini_session(dir.path(), "h", "session-1.json", json_v1);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());

        let (r1, progress) = p.collect_incremental(&ScanProgress::new()).unwrap();
        assert_eq!(r1.messages.len(), 1);
        assert_eq!(r1.sessions[0].title_orig, "first");

        // Rewrite with an added message + bumped lastUpdated (new mtime).
        std::thread::sleep(std::time::Duration::from_millis(20));
        let json_v2 = r#"{"sessionId":"s1","startTime":"2026-08-01T10:00:00Z","lastUpdated":"2026-08-02T12:00:00Z","messages":[{"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"first"},{"id":"m2","type":"user","timestamp":"2026-08-02T12:00:00Z","content":"second"}]}"#;
        std::fs::write(&path, json_v2).unwrap();

        let (r2, _) = p.collect_incremental(&progress).unwrap();
        // Whole file re-parsed: m1 re-emitted + m2 new (ledger dedups at ingest).
        assert_eq!(r2.messages.len(), 2);
        // Meta rebuilt: last_active_at reflects the new lastUpdated.
        assert_eq!(r2.sessions[0].last_active_at, "2026-08-02T12:00:00Z");
    }

    /// Empty content is skipped (string or array with no text blocks); messages
    /// without a stable id are also skipped (dedup needs a key).
    #[test]
    fn gemini_skips_empty_content_and_idless_messages() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"sessionId":"s1","messages":[
            {"id":"e1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":""},
            {"id":"e2","type":"gemini","timestamp":"2026-08-01T10:01:00Z","content":[]},
            {"type":"user","timestamp":"2026-08-01T10:02:00Z","content":"no id"},
            {"id":"ok","type":"user","timestamp":"2026-08-01T10:03:00Z","content":"kept"}
        ]}"#;
        write_gemini_session(dir.path(), "h", "session-1.json", json);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let result = p.parse_full(&p.discover().unwrap()).unwrap();
        // Only the "ok" message survives (non-empty content + has id).
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].uuid, "ok");
        assert_eq!(result.messages[0].content, "kept");
    }

    // ---- session identity declaration: seen set vs fold (mis-delete guard) ----

    /// The seen set reports the JSON `sessionId` (not the stem) — reconcile
    /// deletes stored ids absent from the seen set, so a stem-based seen set
    /// would wipe real gemini sessions.
    #[test]
    fn gemini_seen_set_reports_session_id_not_stem() {
        let dir = tempfile::tempdir().unwrap();
        write_gemini_session(
            dir.path(),
            "hashA",
            "session-1.json",
            r#"{"sessionId":"sess-real","messages":[{"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"hi"}]}"#,
        );
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let files = p.discover().unwrap();
        assert_eq!(p.session_identity().seen(&files), vec!["sess-real"]);
    }

    /// Fallback chain, seen side: a file without `sessionId` (or with an empty
    /// one) falls back to the stem — a fallback can only KEEP a row, never
    /// delete a real session.
    #[test]
    fn gemini_seen_set_falls_back_to_stem_without_session_id() {
        let dir = tempfile::tempdir().unwrap();
        write_gemini_session(
            dir.path(),
            "hashA",
            "session-1.json",
            r#"{"messages":[{"id":"m1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"hi"}]}"#,
        );
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let files = p.discover().unwrap();
        assert_eq!(p.session_identity().seen(&files), vec!["session-1"]);
    }

    /// Consistency by construction, pinned: fold's session id (on the session
    /// row, the usage row, and every message) and the seen set's id agree even
    /// when `sessionId` is missing — the fold used to fill "unknown" here while
    /// the seen set reported the stem, a drift that would let reconcile delete
    /// the session row as a ghost.
    #[test]
    fn gemini_fold_session_id_matches_seen_set_even_without_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let json = r#"{"messages":[
            {"id":"u1","type":"user","timestamp":"2026-08-01T10:00:00Z","content":"hi"},
            {"id":"g1","type":"gemini","timestamp":"2026-08-01T10:01:00Z","model":"gemini-2.5-pro","tokens":{"input":100,"output":10,"cached":0,"thoughts":0}}
        ]}"#;
        write_gemini_session(dir.path(), "hashA", "session-1.json", json);
        let p = GeminiCliSourceParser::with_dir(dir.path().to_path_buf());
        let files = p.discover().unwrap();
        let result = p.parse_full(&files).unwrap();
        assert_eq!(result.sessions.len(), 1);
        assert_eq!(result.sessions[0].id, "session-1", "stem fallback");
        assert!(result.events.iter().all(|e| e.session_id == "session-1"));
        assert!(result.messages.iter().all(|m| m.session_id == "session-1"));
        assert_eq!(p.session_identity().seen(&files), vec!["session-1"]);
    }
}
