//! Project dimension of the model — the shared vocabulary for grouping
//! sessions and usage by launch directory. Split from `session.rs` (架构
//! 审查Ⅶ候选 A3) so the dimension has a nameable home: the sentinel, the
//! dropdown candidates, and the identity rule live together, and the
//! Session/Usage filters reference them without reaching into the session
//! module. Doc comments carried over verbatim.

/// Sentinel for the project dimension's「未知项目」(unknown project) bucket:
/// usage whose session row never arrived. On a multi-device store the only
/// cross-device session rows are pulled FAVORITE snapshots ("收藏才进 git"),
/// so a peer's non-favorited usage resolves to no session — without this
/// bucket that usage would silently vanish from every project view. Session-less
/// legacy usage rows (empty `session_id`) land here too.
///
/// Filter semantics per grain — both are "we cannot name a project for this":
/// - usage side (`UsageFilter.project` = sentinel): NOT EXISTS a `sessions`
///   row for the record's `(session_id, device_id)` — remote usage without a
///   pulled favorite snapshot, or session-less rows;
/// - sessions side (`SessionFilter.project` = sentinel): sessions whose
///   project IDENTITY is empty (`project_identity(s.project_dir) = ''`) — a
///   session row exists, but it carries no launch dir.
///
/// The value crosses the wire as DATA (see [`ProjectCandidates`]), so the
/// frontend labels the special option without a second copy of the literal.
/// A real directory named exactly this string would be mis-bucketed — the
/// double-underscore form is chosen to make that collision pathological.
pub const UNKNOWN_PROJECT: &str = "__unknown_project__";

/// Project-dropdown candidates: the known project identities plus the unknown
/// bucket's presence. Returned by the distinct-projects read; the sentinel
/// rides as data (`unknown`) instead of being embedded in `projects`, so the
/// dropdown can render one labeled special option without recognizing the
/// literal string.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ProjectCandidates {
    /// Known project identities under the filter's other dimensions, sorted.
    /// Never contains the unknown sentinel or the empty identity (a session
    /// with no launch dir is not a pickable project).
    pub projects: Vec<String>,
    /// `Some(sentinel)` when session-less usage exists in the same window —
    /// the「未知项目」option is offered exactly then. `None` = no unknown
    /// usage, so the option stays hidden.
    pub unknown: Option<String>,
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
