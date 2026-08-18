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
    /// Project identity for the project dimension — the stored launch
    /// directory with a Claude Code worktree suffix collapsed to its parent
    /// ([`project_identity`]). The raw launch dir stays in the `sessions` row
    /// and the git snapshot; truncation is a read-side rule, so nothing is
    /// lost and no re-collect is needed for existing rows.
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

/// Map a stored `project_dir` (the session's launch directory, collected raw
/// from the source log) to the project identity the project dimension groups
/// by: a Claude Code worktree suffix — a `.claude` path component immediately
/// followed by a `worktrees` component — collapses to its parent directory.
/// Claude Code launches parallel/subagent sessions inside
/// `<project>/.claude/worktrees/<name>`, transient scratch copies that must
/// aggregate under `<project>` itself (their sessions, tokens, and costs are
/// the parent project's; left raw, each worktree would surface as its own
/// one-session bucket — 15 such buckets on the machine this rule was derived
/// from, issue #84). Both separators match: cwd strings arrive from Windows
/// (`\`) and Unix (`/`) devices alike via the cross-device store. The first
/// worktree segment wins. Returns the input unchanged when no worktree
/// segment exists, or when the cut would leave an empty prefix (a bare
/// relative worktree path keeps its raw form rather than degrading to the
/// empty no-project bucket).
pub fn project_identity(project_dir: &str) -> &str {
    // (byte offset, component) pairs over BOTH separators — a Unix peer's
    // `/home/p/.claude/worktrees/x` and a Windows `\` form live in the same
    // store, so neither separator may be assumed.
    let mut comps: Vec<(usize, &str)> = Vec::new();
    let mut start = 0;
    for (i, c) in project_dir.char_indices() {
        if c == '/' || c == '\\' {
            comps.push((start, &project_dir[start..i]));
            start = i + 1; // '/' and '\\' are 1-byte ASCII
        }
    }
    comps.push((start, &project_dir[start..]));
    for w in 0..comps.len() - 1 {
        if comps[w].1 == ".claude" && comps[w + 1].1 == "worktrees" {
            let (cut, _) = comps[w];
            if cut > 0 {
                // Drop the separator before `.claude` too (also 1-byte).
                return &project_dir[..cut - 1];
            }
        }
    }
    project_dir
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
    /// Scope to one project — matched by project IDENTITY, not the raw stored
    /// launch dir: the comparison runs through the `project_identity` SQL
    /// scalar, so a Claude Code worktree session (`<proj>\.claude\worktrees\…`)
    /// matches its parent project's bucket. Same rule the project aggregate
    /// groups by. `None`/empty = no constraint.
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
    /// original) and the project path — case-insensitive, literal (LIKE
    /// metacharacters are escaped). `None`/empty = no constraint. Lives
    /// backend-side because paged results make client-side filtering
    /// inconsistent (it would only search the loaded page).
    pub search: Option<String>,
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ProjectStatsRow {
    /// Project identity — the bucket key the filter's `project` matches.
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

#[cfg(test)]
mod tests {
    use super::*;

    // ---- project_identity: worktree suffix collapses to the parent project ----

    #[test]
    fn project_identity_collapses_windows_worktree_suffix() {
        // The real-world shape this rule was derived from (issue #84): a
        // subagent/parallel session launched in a Claude Code worktree.
        assert_eq!(
            project_identity("D:\\Project\\O_CC_One\\.claude\\worktrees\\agent-a10c476b"),
            "D:\\Project\\O_CC_One"
        );
    }

    #[test]
    fn project_identity_collapses_unix_worktree_suffix() {
        // A Unix peer's cwd lands in the same cross-device store.
        assert_eq!(
            project_identity("/home/me/proj/.claude/worktrees/agent-ff"),
            "/home/me/proj"
        );
    }

    #[test]
    fn project_identity_no_worktree_segment_is_unchanged() {
        // Ordinary launch dirs — including a project that merely CONTAINS a
        // `.claude` dir (without the `worktrees` child) — pass through.
        assert_eq!(
            project_identity("D:\\Project\\O_CC_One"),
            "D:\\Project\\O_CC_One"
        );
        assert_eq!(project_identity("/home/me/proj"), "/home/me/proj");
        assert_eq!(project_identity("D:\\foo\\.claude"), "D:\\foo\\.claude");
        // A directory whose name merely ends in `.claude` is NOT the segment.
        assert_eq!(
            project_identity("D:\\foo\\my.claude\\worktrees\\x"),
            "D:\\foo\\my.claude\\worktrees\\x"
        );
        assert_eq!(project_identity(""), "");
    }

    #[test]
    fn project_identity_empty_parent_keeps_raw_form() {
        // A bare relative worktree path would truncate to nothing; keeping the
        // raw string avoids degrading the row to the empty no-project bucket.
        assert_eq!(
            project_identity(".claude\\worktrees\\agent-x"),
            ".claude\\worktrees\\agent-x"
        );
    }

    #[test]
    fn project_identity_trailing_separator_and_nested_forms() {
        // Trailing separator: the tail empty component changes nothing.
        assert_eq!(project_identity("/p/.claude/worktrees/agent-x/"), "/p");
        // First segment wins when (pathologically) two appear.
        assert_eq!(
            project_identity("/p/.claude/worktrees/x/.claude/worktrees/y"),
            "/p"
        );
    }
}
