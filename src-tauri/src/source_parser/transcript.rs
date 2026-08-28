//! Shared transcript-message extraction scaffold (架构审查Ⅴ).
//!
//! The five transcript extractors (claude / codex / gemini / grok / opencode)
//! each hand-wrote the same scaffold — content flattening, trim + empty-skip,
//! TRIM_LIMIT truncation, a role dictionary, stable uuid synthesis — and the
//! copies had already drifted: three uuid spellings, an independently-inlined
//! 1024 tool-input cap, two different empty-content policies. This module owns
//! the scaffold once ([`MessageSpec::emit`]); the per-parser differences become
//! declarative data (the same treatment as `GateMode` / `DirectoryShape`):
//!   - [`TextKeys`] — the content-block text rule (optional `type` gate + the
//!     text keys to try, in order);
//!   - `roles` — source role string → UI role dictionary; a source role not in
//!     the dictionary drops the whole line;
//!   - [`MessageUuid`] — the line-uuid synthesis rule (existing formats made
//!     explicit, never migrated).
//!
//! Genuinely different shapes stay in the callers on purpose: claude's fan-out
//! (one event can emit several lines), tool-name extraction (claude block name
//! / gemini `toolCalls` / grok dual keys / opencode tool part / codex
//! `function_call` payload), and timestamp handling. A caller fills one
//! [`RawMessage`] with the raw pieces of one source message; everything else
//! happens here.

use crate::model::{SessionMessage, SessionMessageRole};

use super::{truncate, TRIM_LIMIT};

/// Soft cap on one tool call's displayed input (1024 chars). Previously an
/// inlined magic number in claude and codex independently; one constant so the
/// two inputs' volume contract cannot drift.
pub(super) const TOOL_INPUT_MAX: usize = 1024;

/// Tool input as display text: JSON-serialized, capped at [`TOOL_INPUT_MAX`].
/// Claude's `tool_use.input` (a JSON object) goes through here; codex's
/// `arguments` is already a string in the source and truncates directly.
pub(super) fn tool_input_text(v: &serde_json::Value) -> String {
    truncate(&v.to_string(), TOOL_INPUT_MAX)
}

/// The content-block text rule (declarative): an optional `type` gate plus the
/// text keys to try, in order. One home for what used to be five flatten
/// copies (claude `collect_text` / codex `extract_codex_message_text` / gemini
/// `join_content` / grok `extract_grok_content` / opencode's part walk).
#[derive(Debug, Clone, Copy)]
pub(super) struct TextKeys {
    /// A block counts as a text block only when its `type` field equals this;
    /// `None` = no type gate, try the keys directly (gemini / codex).
    pub block_type: Option<&'static str>,
    /// Text keys tried in order; the first string value wins.
    pub keys: &'static [&'static str],
}

impl TextKeys {
    /// One block's text: type gate first, then the keys in order.
    pub(super) fn block_text(&self, item: &serde_json::Value) -> Option<String> {
        if let Some(want) = self.block_type {
            if item.get("type").and_then(|v| v.as_str()) != Some(want) {
                return None;
            }
        }
        self.keys
            .iter()
            .find_map(|k| item.get(*k).and_then(|v| v.as_str()))
            .map(str::to_string)
    }

    /// Flatten a content value into plain text: a string as-is; an array =
    /// per-block text, blank (trim-empty) blocks dropped, joined with `\n`;
    /// absent / other shapes = empty.
    pub(super) fn flatten(&self, content: Option<&serde_json::Value>) -> String {
        let Some(content) = content else {
            return String::new();
        };
        match content {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Array(items) => items
                .iter()
                .filter_map(|item| self.block_text(item))
                .filter(|s| !s.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        }
    }

    /// First text of a content value: the string itself, or the first matching
    /// block's text (whitespace kept verbatim — the title chain or caller
    /// judges blankness). Used by the original-title candidate paths.
    pub(super) fn first_text(&self, content: Option<&serde_json::Value>) -> Option<String> {
        let content = content?;
        match content {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Array(items) => items.iter().find_map(|item| self.block_text(item)),
            _ => None,
        }
    }
}

/// The line-uuid synthesis rule (declarative).
///
/// Convention (self-contained): every tool's transcript lines on one machine
/// share the single `(uuid, device_id)` key space (`session_messages` dedups
/// by uuid; snapshots rewrite by uuid), so a line uuid must carry a source
/// prefix (`grok:msg:`) or be naturally unique (a source id / source event
/// uuid). **Existing formats stay byte-identical**: the uuid is a persisted
/// primary key — respelling it would re-insert already-stored rows as
/// duplicates. This enum only makes each parser's existing format explicit; it
/// never migrates stored data. Known legacy exception: codex's synthesized
/// form has NO source prefix (`{session_id}:L{line}`) and relies on the
/// session id (a codex thread UUID) being naturally unique — kept as-is.
#[derive(Debug, Clone, Copy)]
pub(super) enum MessageUuid {
    /// The source id is mandatory: absent/empty ⇒ the line is not emitted at
    /// all (gemini / opencode — the dedup key must exist).
    RequiredSourceId,
    /// The source EVENT uuid is mandatory, with a caller suffix appended
    /// (claude: one event can fan out into several lines, `#tool{i}` tags each
    /// block line; the main text line's suffix is empty).
    EventUuid,
    /// Source id preferred; when absent, synthesize
    /// `{prefix}{session_id}{line_sep}{line_no}` (codex legacy: prefix "",
    /// line_sep ":L").
    SourceIdElseLine {
        prefix: &'static str,
        line_sep: &'static str,
    },
    /// Always line-synthesized (grok legacy: prefix "grok:msg:", line_sep
    /// ":line" — its source lines carry no id).
    LineNo {
        prefix: &'static str,
        line_sep: &'static str,
    },
}

/// One parser's message-extraction declaration: the content key table, the
/// role dictionary, and the uuid rule — the three axes that used to be
/// hand-copied per parser, now one value per parser shared by every emit path.
#[derive(Debug, Clone, Copy)]
pub(super) struct MessageSpec {
    /// content 文本块取文规则。
    pub text_keys: TextKeys,
    /// role 词典：源 role 字符串 → UI 角色。未列出的源角色（claude/codex 的
    /// developer、gemini 的 info/error、grok 的 reasoning、opencode 的
    /// system）整条丢弃。
    pub roles: &'static [(&'static str, SessionMessageRole)],
    /// 行 uuid 合成规则（存量格式，见 [`MessageUuid`] 的约定注释）。
    pub uuid_rule: MessageUuid,
}

/// The raw pieces of one source message — everything a caller extracts before
/// the scaffold takes over.
pub(super) struct RawMessage<'a> {
    pub session_id: &'a str,
    /// 源 role 字符串（[`MessageSpec::emit`] 查词典；未列出 ⇒ None）。
    pub role_str: &'a str,
    pub content: Option<&'a serde_json::Value>,
    /// 源时间戳，原样携带（回填策略明确不适用 transcript——ts 是排序键，宁缺勿假，
    /// 见 `fallback_timestamp` 的适用边界）。
    pub ts: &'a str,
    pub model: Option<String>,
    pub name: Option<String>,
    /// 源自带 id（可能缺席；后果由 [`MessageUuid`] 规则裁定）。
    pub source_id: Option<&'a str>,
    /// 源内 1-based 行号（行号合成规则消费；非行源传 0）。
    pub line_no: i64,
    /// uuid 后缀（claude 扇出行的 `#tool{i}`；其余为空）。
    pub uuid_suffix: &'a str,
}

impl MessageSpec {
    /// Dictionary lookup: source role string → UI role; not listed ⇒ None
    /// (the whole line is dropped as noise).
    pub(super) fn map_role(&self, role_str: &str) -> Option<SessionMessageRole> {
        self.roles
            .iter()
            .find(|(s, _)| *s == role_str)
            .map(|(_, r)| *r)
    }

    /// Main entry: role dictionary → flatten → trim → empty-skip → TRIM_LIMIT
    /// truncation → uuid synthesis → [`SessionMessage`]. Any step failing
    /// yields `None` — the caller skips the line (a filter, not an error).
    pub(super) fn emit(&self, m: RawMessage<'_>) -> Option<SessionMessage> {
        self.emit_as(self.map_role(m.role_str)?, m)
    }

    /// Fan-out entry: the caller has already decided the role (claude's
    /// `tool_use` block → a Tool line) and skips the dictionary, entering the
    /// same tail steps as [`MessageSpec::emit`].
    pub(super) fn emit_as(
        &self,
        role: SessionMessageRole,
        m: RawMessage<'_>,
    ) -> Option<SessionMessage> {
        // 空文本过滤（统一决策，五源同口径）：拍平层丢弃 trim 空白的文本块；
        // 最终文本 trim 后为空 ⇒ 整条不发射。空行没有信息量，只会在按
        // (ts, uuid) 排序的 transcript 里稀释上下文；此前 codex 滤空白块而
        // claude/gemini 不滤、grok/claude 不 trim 最终文本的漂移在此收口。
        let joined = self.text_keys.flatten(m.content);
        let text = truncate(joined.trim(), TRIM_LIMIT);
        if text.is_empty() {
            return None;
        }
        Some(SessionMessage {
            uuid: self.synthesize_uuid(&m)?,
            session_id: m.session_id.to_string(),
            role,
            ts: m.ts.to_string(),
            model: m.model,
            name: m.name,
            content: text,
        })
    }

    /// uuid synthesis per the declared rule; `None` = no legal key for this
    /// line, so it is not emitted.
    fn synthesize_uuid(&self, m: &RawMessage<'_>) -> Option<String> {
        match self.uuid_rule {
            MessageUuid::RequiredSourceId => {
                m.source_id.filter(|s| !s.is_empty()).map(str::to_string)
            }
            MessageUuid::EventUuid => Some(format!(
                "{}{}",
                m.source_id.filter(|s| !s.is_empty())?,
                m.uuid_suffix
            )),
            MessageUuid::SourceIdElseLine { prefix, line_sep } => Some(
                m.source_id
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{prefix}{}{line_sep}{}", m.session_id, m.line_no)),
            ),
            MessageUuid::LineNo { prefix, line_sep } => {
                Some(format!("{prefix}{}{line_sep}{}", m.session_id, m.line_no))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(
        text_keys: TextKeys,
        roles: &'static [(&'static str, SessionMessageRole)],
        uuid_rule: MessageUuid,
    ) -> MessageSpec {
        MessageSpec {
            text_keys,
            roles,
            uuid_rule,
        }
    }

    fn raw<'a>(
        role_str: &'a str,
        content: Option<&'a serde_json::Value>,
        source_id: Option<&'a str>,
        line_no: i64,
    ) -> RawMessage<'a> {
        RawMessage {
            session_id: "s1",
            role_str,
            content,
            ts: "2026-08-01T10:00:00Z",
            model: None,
            name: None,
            source_id,
            line_no,
            uuid_suffix: "",
        }
    }

    // ---- TextKeys::flatten ----

    #[test]
    fn flatten_handles_string_typed_and_ungated_arrays() {
        let typed = TextKeys {
            block_type: Some("text"),
            keys: &["text"],
        };
        // String content passes through.
        assert_eq!(typed.flatten(Some(&json!("plain"))), "plain");
        // Type-gated blocks: non-text types contribute nothing.
        let arr = json!([
            {"type":"text","text":"Hello"},
            {"type":"thinking","text":"secret"},
            {"type":"text","text":"World"}
        ]);
        assert_eq!(typed.flatten(Some(&arr)), "Hello\nWorld");
        // Ungated keys (gemini shape): every item's "text" key counts.
        let ungated = TextKeys {
            block_type: None,
            keys: &["text"],
        };
        assert_eq!(ungated.flatten(Some(&arr)), "Hello\nsecret\nWorld");
        // Multi-key lookup, first hit wins (codex shape).
        let multi = TextKeys {
            block_type: None,
            keys: &["text", "input_text", "output_text"],
        };
        let codex_arr = json!([{"output_text":"Out"}, {"input_text":"In"}]);
        assert_eq!(multi.flatten(Some(&codex_arr)), "Out\nIn");
        // Absent / object / null shapes flatten to empty.
        assert_eq!(typed.flatten(None), "");
        assert_eq!(typed.flatten(Some(&json!({"a":1}))), "");
        assert_eq!(typed.flatten(Some(&serde_json::Value::Null)), "");
    }

    #[test]
    fn flatten_drops_blank_blocks_but_keeps_inner_text() {
        let typed = TextKeys {
            block_type: Some("text"),
            keys: &["text"],
        };
        let arr = json!([
            {"type":"text","text":"   "},
            {"type":"text","text":"real"},
            {"type":"text","text":"\n\t "}
        ]);
        assert_eq!(
            typed.flatten(Some(&arr)),
            "real",
            "blank blocks are dropped, not joined"
        );
    }

    #[test]
    fn first_text_returns_first_match_verbatim() {
        let typed = TextKeys {
            block_type: Some("text"),
            keys: &["text"],
        };
        let arr = json!([
            {"type":"thinking","text":"hidden"},
            {"type":"text","text":"  padded  "},
            {"type":"text","text":"second"}
        ]);
        assert_eq!(
            typed.first_text(Some(&arr)).as_deref(),
            Some("  padded  "),
            "first matching block, whitespace verbatim"
        );
        assert_eq!(
            typed.first_text(Some(&json!("str"))).as_deref(),
            Some("str")
        );
        assert!(typed.first_text(None).is_none());
        // A blank FIRST block still wins (the caller judges blankness — this
        // is the title-candidate path's contract).
        let blank_first = json!([{"type":"text","text":" "},{"type":"text","text":"x"}]);
        assert_eq!(typed.first_text(Some(&blank_first)).as_deref(), Some(" "));
    }

    // ---- emit: role dictionary / empty policy / truncation ----

    static ROLES: &[(&str, SessionMessageRole)] = &[
        ("user", SessionMessageRole::User),
        ("assistant", SessionMessageRole::Assistant),
        ("gemini", SessionMessageRole::Assistant),
    ];

    #[test]
    fn emit_maps_roles_and_drops_unlisted_ones() {
        let s = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::RequiredSourceId,
        );
        let m = s
            .emit(raw("gemini", Some(&json!("hi")), Some("id1"), 0))
            .unwrap();
        assert_eq!(m.role, SessionMessageRole::Assistant);
        // "info" is not in the dictionary ⇒ dropped.
        assert!(s
            .emit(raw("info", Some(&json!("x")), Some("id2"), 0))
            .is_none());
        assert!(s.emit(raw("", Some(&json!("x")), Some("id3"), 0)).is_none());
    }

    #[test]
    fn emit_trims_skips_blank_and_truncates() {
        let s = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::RequiredSourceId,
        );
        // Whitespace-only content ⇒ no line.
        assert!(s
            .emit(raw("user", Some(&json!("   ")), Some("id"), 0))
            .is_none());
        // Surrounding whitespace trimmed into the stored text.
        let m = s
            .emit(raw("user", Some(&json!("  hello  ")), Some("id"), 0))
            .unwrap();
        assert_eq!(m.content, "hello");
        // Over TRIM_LIMIT: truncated with the shared ellipsis.
        let long = "x".repeat(TRIM_LIMIT + 100);
        let m = s
            .emit(raw("user", Some(&json!(long)), Some("id"), 0))
            .unwrap();
        assert!(m.content.ends_with('…'));
        assert!(m.content.chars().count() <= TRIM_LIMIT);
        // passthrough fields ride along.
        let content = json!("hi");
        let mut with_meta = raw("user", Some(&content), Some("id"), 0);
        with_meta.model = Some("glm-5.2".into());
        with_meta.name = Some("Read".into());
        let m = s.emit(with_meta).unwrap();
        assert_eq!(m.model.as_deref(), Some("glm-5.2"));
        assert_eq!(m.name.as_deref(), Some("Read"));
        assert_eq!(m.session_id, "s1");
        assert_eq!(m.ts, "2026-08-01T10:00:00Z");
    }

    // ---- uuid rules (existing formats, pinned byte-for-byte) ----

    #[test]
    fn uuid_required_source_id_skips_idless_lines() {
        let s = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::RequiredSourceId,
        );
        assert!(s.emit(raw("user", Some(&json!("x")), None, 0)).is_none());
        assert!(s
            .emit(raw("user", Some(&json!("x")), Some(""), 0))
            .is_none());
        assert_eq!(
            s.emit(raw("user", Some(&json!("x")), Some("m1"), 0))
                .unwrap()
                .uuid,
            "m1"
        );
    }

    #[test]
    fn uuid_event_uuid_appends_suffix_and_requires_the_id() {
        let s = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::EventUuid,
        );
        let content = json!("x");
        let mut m = raw("user", Some(&content), Some("ev1"), 0);
        m.uuid_suffix = "#tool3";
        assert_eq!(s.emit(m).unwrap().uuid, "ev1#tool3");
        // Missing event uuid ⇒ no line (claude's contract).
        assert!(s.emit(raw("user", Some(&content), None, 0)).is_none());
    }

    #[test]
    fn uuid_source_id_else_line_keeps_legacy_spellings() {
        // codex legacy: no prefix, ":L" separator.
        let s = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::SourceIdElseLine {
                prefix: "",
                line_sep: ":L",
            },
        );
        assert_eq!(
            s.emit(raw("user", Some(&json!("x")), Some("pid"), 7))
                .unwrap()
                .uuid,
            "pid"
        );
        assert_eq!(
            s.emit(raw("user", Some(&json!("x")), None, 7))
                .unwrap()
                .uuid,
            "s1:L7"
        );
        // grok legacy: "grok:msg:" prefix, ":line" separator, never source-id.
        let g = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::LineNo {
                prefix: "grok:msg:",
                line_sep: ":line",
            },
        );
        assert_eq!(
            g.emit(raw("user", Some(&json!("x")), Some("ignored"), 4))
                .unwrap()
                .uuid,
            "grok:msg:s1:line4"
        );
    }

    // ---- emit_as (fan-out entry) + tool input cap ----

    #[test]
    fn emit_as_bypasses_the_dictionary_but_shares_the_tail() {
        let s = spec(
            TextKeys {
                block_type: None,
                keys: &["text"],
            },
            ROLES,
            MessageUuid::RequiredSourceId,
        );
        // An unlisted role string still emits when the caller decided the role.
        let content = json!("{}");
        let mut m = raw("tool_use", Some(&content), Some("ev1"), 0);
        m.name = Some("Bash".into());
        let line = s.emit_as(SessionMessageRole::Tool, m).unwrap();
        assert_eq!(line.role, SessionMessageRole::Tool);
        assert_eq!(line.name.as_deref(), Some("Bash"));
        // The tail still applies: blank content is skipped.
        let blank = json!(" ");
        assert!(s
            .emit_as(
                SessionMessageRole::Tool,
                raw("tool_use", Some(&blank), Some("e"), 0)
            )
            .is_none());
    }

    #[test]
    fn tool_input_text_serializes_and_caps() {
        let v = json!({"cmd": ["ls", "-la"], "n": 1});
        let text = tool_input_text(&v);
        assert!(text.contains("\"cmd\""));
        let huge = json!({"x": "y".repeat(TOOL_INPUT_MAX * 4)});
        assert!(
            tool_input_text(&huge).chars().count() <= TOOL_INPUT_MAX,
            "capped at the shared constant"
        );
    }
}
