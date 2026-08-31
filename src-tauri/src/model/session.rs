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

use super::usage::UsageFilter;

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
    /// Parent link for a subagent session: the id of the main session it was
    /// spawned under, `""` when none. Claude Code's own placement is the
    /// explicit source — it writes subagent files at
    /// `<project>/<parent-session-id>/subagents/agent-*.jsonl`, so the parent
    /// is read off the directory chain (no heuristic: the placement is written
    /// by Claude Code, and every nested agent file pairs with a real parent
    /// session file). Older Claude Code versions wrote agent files flat in the
    /// project dir — no link exists there, so those rows keep `""` (top-level
    /// display, `agent_type` still set). Display-only structural ownership:
    /// the sessions stay separate rows (details are never merged), and tokens
    /// never move between them.
    pub parent_session_id: String,
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
    /// Parent link (subagent → its main session's id; `""` = none), same
    /// semantics as the sessions row's `parent_session_id`. `serde(default)`
    /// keeps pre-field snapshots parsing — and an older peer's binary simply
    /// ignores the extra key, so the version stays compatible both ways (no
    /// `SESSION_SNAPSHOT_VERSION` bump).
    #[serde(default)]
    pub parent_session_id: String,
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
    /// Project identity for the project dimension — the stored launch
    /// directory with a Claude Code worktree suffix collapsed to its parent
    /// ([`project_identity`](crate::model::project_identity)). The raw launch
    /// dir stays in the `sessions` row
    /// and the git snapshot; truncation is a read-side rule, so nothing is
    /// lost and no re-collect is needed for existing rows.
    pub project_dir: String,
    /// Display title: `custom_title` when set, else `title_orig`.
    pub title: String,
    /// `""` = main session; non-empty = subagent type tag (e.g. `Explore`).
    pub agent_type: String,
    /// Parent link (subagent → its main session's id on the SAME device;
    /// `""` = no parent / top-level). Drives the indented "child" placement
    /// under the parent row in the workbench's session list — display
    /// structure only, the rows stay separate (see
    /// `SessionSystemData::parent_session_id`).
    pub parent_session_id: String,
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

/// A session's composite key `(id, device_id)` — the addressing form every
/// session write already takes as two args, lifted into a DTO for the ONE
/// place that addresses many sessions at once: the batch soft-delete
/// ([`Store::delete_sessions`]). A session is uniquely `(device_id, id)` —
/// the same id can exist on two devices — so batch operations key on the pair,
/// never the bare id.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionKey {
    pub id: String,
    pub device_id: String,
}

/// Optional filter for `query_sessions`. Every field optional; `None` = no
/// constraint. The shared-facet half mirrors `UsageFilter` exactly — the one
/// cross-grain mapping between the two shapes lives on
/// [`SessionFilter::to_usage_grain`] / [`UsageFilter::to_session_grain`].
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
    /// Scope to one project — matched by project IDENTITY, not the raw stored
    /// launch dir: the comparison runs through the `project_identity` SQL
    /// scalar, so a Claude Code worktree session (`<proj>\.claude\worktrees\…`)
    /// matches its parent project's bucket. Same rule the project aggregate
    /// groups by. The [`UNKNOWN_PROJECT`](crate::model::UNKNOWN_PROJECT) sentinel matches sessions whose
    /// identity is EMPTY (a session row exists but carries no launch dir) —
    /// the sessions-side face of the unknown bucket; the usage-side face
    /// (session-less usage, NOT EXISTS) lives on `UsageFilter.project`.
    /// `None`/empty = no constraint.
    pub project: Option<String>,
    /// Inclusive lower bound on `last_active_at` (ISO8601).
    pub from_ts: Option<String>,
    /// Inclusive upper bound on `last_active_at` (ISO8601).
    pub to_ts: Option<String>,
    /// Scope to sessions that have at least one usage record with this model
    /// (EXISTS semantics — a session spanning several models matches any of
    /// them). The model lives on `usage_records`, not on the session row.
    pub model: Option<String>,
    /// Substring search over the display title (custom title when set, else the
    /// original), the project path, and every message body
    /// (`session_messages.content`, probed across ALL devices of the session —
    /// the same union the merged transcript read shows) — case-insensitive,
    /// literal (LIKE metacharacters are escaped). `None`/empty = no constraint.
    /// Lives backend-side because paged results make client-side filtering
    /// inconsistent (it would only search the loaded page).
    pub search: Option<String>,
}

impl SessionFilter {
    /// The usage-grain view of this filter — the five fields the two filter
    /// shapes share (time / device / model / source). The unknown bucket's
    /// direct read goes through the SAME usage-grain assembly as every other
    /// usage read (`push_usage_facets`), which takes the `UsageFilter` shape;
    /// this and the reverse [`UsageFilter::to_session_grain`] are the one
    /// seam between the two grains. 字段穷举而非 `..Default::default()`：
    /// 给 `UsageFilter` 新增字段会让这个字面量编译失败——漏接未知桶在
    /// 这里被编译器拦下，而不是静默漂移。`project` 恒 `None`：桶的
    /// 「项目」就是其 NOT EXISTS 定义，不是一层筛选；known 桶的项目语义
    /// 归 build_session_where。
    pub fn to_usage_grain(&self) -> UsageFilter {
        UsageFilter {
            from_ts: self.from_ts.clone(),
            to_ts: self.to_ts.clone(),
            model: self.model.clone(),
            source: self.source.clone(),
            device_scope: self.device_scope.clone(),
            project: None,
        }
    }
}

/// Paged session-list query — mirrors `LogsQuery` (filter + limit + offset) so
/// the sessions table paginates like the request log instead of loading every
/// row into the UI. `offset` is an absolute row offset into the filtered,
/// time-desc ordered set.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionQuery {
    pub filter: Option<SessionFilter>,
    pub limit: u32,
    pub offset: u32,
}

/// The two session grouping tracks — which group column a read (sidebar
/// counts) or write (group CRUD) addresses. `Local` groups live in
/// device-private SQLite (`local_group_id`); `Synced` groups ride the
/// per-device `groups.json` via git (`synced_group_id`). An enum, not a
/// string: the column behind each track is a fixed map inside the store, and
/// a mistyped track name should be unrepresentable across the boundary — not
/// a runtime-rejected magic string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum GroupTrack {
    Local,
    Synced,
}

/// One sidebar group bucket's session count under the current filter — the
/// `group_id` is the track's group column value (empty string = ungrouped; a
/// stale id whose group was deleted counts toward ungrouped, resolved
/// client-side against the known group list).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionGroupCount {
    pub group_id: String,
    pub count: u32,
}

/// Sidebar counts for one grouping track under a filter: the total (drives the
/// "All" row + the paginator) plus per-bucket counts. Independent of paging —
/// it describes the whole filtered set, so the sidebar numbers stay correct
/// while the table shows one page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionGroupCounts {
    pub total: u32,
    pub groups: Vec<SessionGroupCount>,
}

/// One project bucket of the project dimension: every session whose project
/// IDENTITY equals `project_dir`, rolled up with its usage. Session count and
/// last-active come from `sessions`; requests / token buckets / cost are live
/// aggregates over `usage_records` (the same single source of token truth
/// `SessionRow` reads — nothing is stored at project grain). The bucket key is
/// the project identity (the `project_identity` SQL scalar = the #84 rule), so
/// a Claude Code worktree session and its usage land under the PARENT project,
/// never as a one-session bucket of their own.
///
/// One SYNTHETIC row may carry the [`UNKNOWN_PROJECT`](crate::model::UNKNOWN_PROJECT) sentinel as its key
/// instead: the aggregate over usage with no session row (remote usage whose
/// favorite snapshot was never pulled, session-less legacy rows). Its
/// `session_count` is 0 by definition (no session rows exist) and its
/// `last_active_at` is the MAX usage timestamp, so the bucket sorts by real
/// recency.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ProjectStatsRow {
    /// Project identity — the bucket key the filter's `project` matches. One
    /// synthetic row can carry the [`UNKNOWN_PROJECT`](crate::model::UNKNOWN_PROJECT) sentinel instead: the
    /// aggregate over session-less usage (see `Store::query_project_stats`).
    pub project_dir: String,
    /// Sessions in the bucket. Same grain as the session list: one row per
    /// (session id, device), so a session collected on two devices counts
    /// twice.
    pub session_count: u32,
    /// Live aggregate: usage rows across the bucket's sessions.
    pub request_count: u32,
    /// Live aggregate: sum of all four token buckets
    /// ([`TokenCounts::total`], computed at decode time).
    pub total_tokens: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    /// Cache-hit ratio over the bucket's cacheable pool, [0,1]
    /// ([`TokenCounts::cache_hit_rate`], computed at decode time — the same
    /// single implementation the dashboard and per-model rows use).
    pub cache_hit_rate: f64,
    /// Live aggregate: sum of cost.
    pub total_cost_usd: f64,
    /// `MAX(last_active_at)` across the bucket's sessions (ISO8601) — drives
    /// the bucket ordering.
    pub last_active_at: String,
}

/// One (model, total-tokens) share within a [`SessionStatsRow`] — the per-model
/// slice the sessions workbench's model card renders. Tokens are the model's
/// four-bucket sum within that session; the share denominator is the session's
/// total (the frontend derives the percentage).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionModelTokens {
    /// Model name as usage_records store it (empty when a session has usage
    /// rows with no model — kept as its own slice so the sums still add up).
    pub model: String,
    pub tokens: u32,
}

/// One session of the stats dimension: the session-list row's identity fields
/// plus the usage aggregates the sessions workbench's right rail needs at
/// SESSION grain (the project grain lives in [`ProjectStatsRow`]). Four-bucket
/// totals / hit rate / cost are live aggregates over `usage_records` (the same
/// single token source the list reads); `message_count` counts
/// `session_messages` rows; `models` splits the session's tokens per model.
/// Sessions with NO usage still appear (LEFT JOIN + COALESCE) — the rail's
/// usage card shows zeros, and the tree counts the session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct SessionStatsRow {
    pub id: String,
    pub device_id: String,
    pub source: String,
    /// Project identity (worktree suffix collapsed — same decode rule as
    /// [`SessionRow::project_dir`]).
    pub project_dir: String,
    /// Display title: `custom_title` when set, else `title_orig`.
    pub title: String,
    /// `""` = main session; non-empty = subagent type tag.
    pub agent_type: String,
    pub favorited: bool,
    pub local_group_id: String,
    pub synced_group_id: String,
    pub started_at: String,
    pub last_active_at: String,
    /// Live aggregate over `usage_records`.
    pub request_count: u32,
    /// Rows in `session_messages` for this (session, device).
    pub message_count: u32,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_creation_tokens: u32,
    pub cache_read_tokens: u32,
    /// Cache-hit ratio, [0,1] ([`TokenCounts::cache_hit_rate`]).
    pub cache_hit_rate: f64,
    /// Live aggregate: sum of cost.
    pub total_cost_usd: f64,
    /// Per-model token split, most-tokens-first.
    pub models: Vec<SessionModelTokens>,
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
