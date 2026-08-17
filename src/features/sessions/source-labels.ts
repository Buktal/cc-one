import type { SourceTag } from "@/lib/source-tags"

// Session source tag → display label. Single source for the sessions feature so
// the list table, the detail sheet's source line, and the source-filter dropdown
// all agree (architecture.md: 单一事实来源 — previously two copies, one in
// sessions-view and one in session-detail-sheet, had drifted into a third use
// site).
//
// `source` is the stable source tag on every session row (e.g. "claude_code").
// Sessions use the full product name incl. "CLI" — unlike the usage view
// (features/usage/source-labels.ts, short "Codex"/"Grok"), a session row carries
// no extra context to disambiguate. Unknown tags fall through verbatim so a new
// source shows up before a mapping is added; an empty tag shows "—".

const SESSION_SOURCE_LABELS: Record<SourceTag, string> = {
  claude_code: "Claude Code",
  codex_cli: "Codex CLI",
  gemini_cli: "Gemini CLI",
  grok_cli: "Grok CLI",
  opencode: "OpenCode",
}

/** Map a session source tag to its display name; unknown tags verbatim, empty → "—". */
export function sessionSourceLabel(source: string): string {
  // 查表入口以 string 索引（未知 tag 原样回退）；Record<SourceTag,…> 的键集
  // 完整性由类型守住——新增 SOURCE_TAGS 而漏译时这里编译失败。
  const lookup: Partial<Record<string, string>> = SESSION_SOURCE_LABELS
  return lookup[source] ?? (source || "—")
}

// ---- Session type (main vs subagent) ----
//
// `agent_type` on a session row: `""` = a main (user) session; non-empty = a
// subagent session holding the agent type from its `.meta.json` (e.g.
// "Explore", or the generic "agent" fallback). The backend stores the raw tag;
// this helper classifies it into the two kinds the UI names, so the
// main/subagent split lives in one place and the row template only localizes
// the labels.

export type SessionAgentKind =
  | { kind: "main" }
  | { kind: "subagent"; type: string }

/** Classify a session row's `agent_type` tag into a displayable kind. */
export function sessionAgentKind(agentType: string): SessionAgentKind {
  return agentType ? { kind: "subagent", type: agentType } : { kind: "main" }
}
