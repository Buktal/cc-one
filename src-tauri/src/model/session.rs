//! Session management types — session-as-usage-grouping-key.
//!
//! Session is a grouping key on `usage_records`, NOT a parallel entity. These
//! types model the two layers a session carries:
//!   - system data (collect can re-extract and refresh freely), and
//!   - user data (custom_title / favorited / group_id — re-extract MUST NOT
//!     overwrite; the SQLite UPSERT policy enforces this invariant in code).
//!
//! `local_group_id` is local-only (never enters git); the syncable user data
//! (`custom_title` / `favorited` / `synced_group_id`) rides the sessions table
//! row, which the per-session sync phase will carry into git.

/// Session system data: the layer collect re-extracts from the source log on
/// every pass. Refreshable — re-collecting a session updates these fields in
/// place. This is a strict subset of the SQLite `sessions` row (which also
/// adds the user-data columns and the local-only `local_group_id`).
///
/// Also serves as the parser output type alias [`RawSession`] — there is no
/// device/cost attaching step for sessions (unlike `RawUsage` → `UsageRecord`),
/// so the parser-output shape and the system-data layer are identical. One
/// struct, one source of truth.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub struct SessionSystemData {
    /// Session id (Claude = the jsonl file stem).
    pub id: String,
    /// Source tag, e.g. `claude_code`.
    pub source: String,
    /// Working directory the session ran in (Claude `cwd`).
    pub project_dir: String,
    /// Best-effort original title (Claude `summary` / first user message;
    /// subagent sessions use the task `description` from their `.meta.json`).
    pub title_orig: String,
    /// ISO8601 of the first event observed in the source log.
    pub started_at: String,
    /// ISO8601 of the most recent event observed. Drives session-list ordering.
    pub last_active_at: String,
    /// Agent type tag: `""` = a main (user) session; non-empty = a subagent
    /// session, holding the agent type from its `.meta.json` (e.g. `Explore`).
    /// Unknown types fall back to `"agent"`. Drives the type column in the
    /// session list ("main" vs "subagent(Explore)").
    pub agent_type: String,
}

/// Parser-output alias for a parsed session (pre-device). Identical to
/// [`SessionSystemData`] — no device/cost step exists for sessions, so the two
/// concepts share one struct (single source of truth).
pub type RawSession = SessionSystemData;

/// Role of a transcript line. Matches Claude Code's event types, collapsed to
/// the four values the UI reasons about.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum SessionMessageRole {
    #[default]
    User,
    Assistant,
    Tool,
    System,
}

impl SessionMessageRole {
    /// The lowercase string form persisted in `session_messages.role` and used
    /// everywhere a role crosses a text boundary. Kept in lockstep with the
    /// `#[serde(rename_all = "lowercase")]` mapping above so the DB, the JSONL
    /// snapshot, and serde all agree on one spelling per variant.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionMessageRole::User => "user",
            SessionMessageRole::Assistant => "assistant",
            SessionMessageRole::Tool => "tool",
            SessionMessageRole::System => "system",
        }
    }

    /// Inverse of [`Self::as_str`]. An unknown string defaults to `User` (the
    /// enum default) rather than failing — a malformed stored row should not
    /// crash a transcript read.
    #[allow(dead_code)] // unused while transcript reads still come from the jsonl artifact (see Store::query_session_messages)
    pub fn parse_str(s: &str) -> Self {
        match s {
            "assistant" => SessionMessageRole::Assistant,
            "tool" => SessionMessageRole::Tool,
            "system" => SessionMessageRole::System,
            _ => SessionMessageRole::User,
        }
    }
}

/// One transcript line. Single source of truth across three roles: parser
/// output, the per-session JSONL Artifact (`sessions/<id>.jsonl`), and the DTO
/// crossing to the frontend. The shape is identical for all three, so one
/// struct (single source of truth) — a parser-emitted message is this same
/// shape, not a separate type.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionMessage {
    /// Source event uuid (dedup key within one session's transcript file).
    pub uuid: String,
    /// Session this message belongs to.
    pub session_id: String,
    pub role: SessionMessageRole,
    /// ISO8601 timestamp of the source event.
    pub ts: String,
    /// Model on assistant messages (None for user/tool/system).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Tool name on tool_use messages (None otherwise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Trimmed text content: text blocks for user/assistant, the tool_use `name`
    /// summary for tool calls; thinking blocks' full text, base64 images, and
    /// >32 KB tool_results are filtered/truncated at collect time.
    pub content: String,
}

/// Format version of a session snapshot Artifact (`sessions/<id>.jsonl`). Bumped
/// only on an incompatible line-shape change; pull refuses a snapshot whose
/// version is higher than the running binary supports rather than importing
/// partially-understood data and corrupting state.
pub const SESSION_SNAPSHOT_VERSION: u32 = 1;

/// The meta line of a session snapshot — always the FIRST line of
/// `sessions/<id>.jsonl`. Carries the session's system data plus the two
/// favorites-track user fields (`favorited`, `synced_group_id`) so a peer that
/// pulls the snapshot can reconstruct the full row (the favorites tab is
/// cross-device: a peer's favorited session must surface with its title and
/// group, not just its messages). `v` is the upgrade gate — readers refuse a
/// version higher than they support.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionSnapshotMeta {
    pub v: u32,
    pub id: String,
    pub source: String,
    pub project_dir: String,
    pub title_orig: String,
    pub started_at: String,
    pub last_active_at: String,
    /// Subagent type tag (`""` = main session), same semantics as the sessions
    /// row's `agent_type`. `serde(default)` so snapshots written before this
    /// field existed still parse (they project as main sessions).
    #[serde(default)]
    pub agent_type: String,
    pub favorited: bool,
    pub synced_group_id: String,
}

/// One line of a session snapshot Artifact. The first line is always
/// [`SessionSnapshotLine::Session`]; the rest are
/// [`SessionSnapshotLine::Message`] in `(ts, uuid)` order. The tagged enum makes
/// every line self-describing — the pull reader dispatches on `type` rather than
/// trusting line position, so a future shape change only has to add a variant
/// and bump `v`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SessionSnapshotLine {
    Session(SessionSnapshotMeta),
    Message(SessionMessage),
}

/// One session row for the frontend list. Aggregates (request_count /
/// total_tokens / total_cost_usd) are computed live by `GROUP BY session_id`
/// over `usage_records` at query time — they are NOT stored on the session, so
/// there is no second source of token/cost truth to drift.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionRow {
    pub id: String,
    pub device_id: String,
    pub source: String,
    pub project_dir: String,
    /// Display title: `custom_title` when set, else `title_orig`.
    pub title: String,
    /// `""` = main session; non-empty = subagent type tag (e.g. `Explore`).
    pub agent_type: String,
    pub favorited: bool,
    pub local_group_id: String,
    pub synced_group_id: String,
    pub started_at: String,
    pub last_active_at: String,
    /// Live aggregate over `usage_records` for this session.
    pub request_count: u32,
    /// Live aggregate: sum of all four token buckets.
    pub total_tokens: u32,
    /// Live aggregate: sum of cost.
    pub total_cost_usd: f64,
}

/// Optional filter for `query_sessions`. Every field optional; `None` = no
/// constraint. Mirrors the shape of `UsageFilter` for the session list.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionFilter {
    /// Scope to one device (`None` = all devices).
    pub device_scope: Option<String>,
    /// Scope to one source, e.g. `claude_code`.
    pub source: Option<String>,
    /// `Some(true)` = only favorited; `Some(false)` = only non-favorited.
    pub favorited: Option<bool>,
    /// Scope to a local group (empty string matches ungrouped).
    pub local_group_id: Option<String>,
    /// Scope to a synced group (empty string matches ungrouped).
    pub synced_group_id: Option<String>,
    /// Inclusive lower bound on `last_active_at` (ISO8601).
    pub from_ts: Option<String>,
    /// Inclusive upper bound on `last_active_at` (ISO8601).
    pub to_ts: Option<String>,
    /// Scope to sessions that have at least one usage record with this model
    /// (EXISTS semantics — a session spanning several models matches any of
    /// them). The model lives on `usage_records`, not on the session row.
    pub model: Option<String>,
}

/// One group entry for the frontend, unified across the two tracks. Order is
/// carried by the ARRAY order `list_groups_dto` returns (already sorted by
/// position per track) — no redundant per-row sort key.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionGroup {
    pub id: String,
    pub name: String,
    /// `"local"` (device-private SQLite) or `"synced"` (per-device groups.json).
    pub kind: String,
    /// Owning device id. Only meaningful for `kind == "synced"`; empty for local.
    pub device_id: String,
}

/// A local group row (SQLite `local_groups`; device-private, never enters git).
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub struct LocalGroup {
    pub id: String,
    pub name: String,
    pub created_at: String,
    /// Sort key within the track. `list_local_groups` orders by it (ties fall
    /// back to name, keeping the pre-sort-order output deterministic).
    pub position: u32,
}

/// Missing-position fallback for old synced-groups files (no `position` field):
/// MAX sorts them AFTER user-ordered groups instead of jumping to the front.
pub fn default_group_position() -> u32 {
    u32::MAX
}

/// A synced-group row (`data/<deviceId>/groups.json`; cross-device via git).
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub struct SyncedGroup {
    pub id: String,
    pub name: String,
    /// Owning device (the one that created the group). Encoded in the id prefix
    /// too, but kept here for read-without-parse convenience.
    pub device_id: String,
    pub updated_at: String,
    /// User-ordered position WITHIN this device's own groups (array index order
    /// can't survive the per-device merge, so the rank is explicit data). Old
    /// files lack the field — `default_group_position` (MAX) sorts them last.
    #[serde(default = "default_group_position")]
    pub position: u32,
}
