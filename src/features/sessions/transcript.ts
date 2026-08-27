// Pure derivations of the transcript view itself — the detail sheet's
// row-level presentation rules: whether a row defaults to collapsed and its
// open state (the one xor rule both the per-row read end and the bulk-toggle
// write end hang off), the bulk collapse / expand end states, transcript-wide
// substring search with its snippet windows, and the small display shorthands
// the rows share (first line as a one-line label, JSON pretty-printing for
// tool payloads). Sits beside the transcript domain's other modules —
// turn-nav.ts (turn anchors), turn-search.ts (the nav panel's search state
// machine), conversation.ts (turn grouping / tool attachment), highlight.tsx
// (hit highlighting) — each owning one concern; this one owns row-level
// presentation state. Pure functions so each rule is testable in isolation
// (architecture.md: "关键不变量用代码表达").

import type { SessionMessage } from "@/types/generated/bindings"

// ------------------------------------------------- one-line display helpers --

/**
 * Pretty-print `text` when it is a JSON document — tool rows carry the tool_use
 * input as a JSON string, and 2-space indentation turns that one-liner into
 * something skimmable. Only objects / arrays qualify (a bare `"abc"` or `123`
 * is valid JSON but formatting it adds nothing). Returns null when `text` is
 * not JSON — callers then render it as plain text.
 */
export function tryFormatJson(text: string): string | null {
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    return null
  }
  if (typeof value !== "object" || value === null) return null
  return JSON.stringify(value, null, 2)
}

/** First line of a message — the one-line label for a tool row header and the
 *  turn-nav panel row. Empty text yields "". */
export function firstLine(text: string): string {
  return text.split("\n")[0]
}

// ------------------------------------------------------- row collapse state --

/** Does a message role default to collapsed? The xor rule — collapsed-set
 *  membership means the OPPOSITE of the row's default: messages default
 *  expanded (open = not-in-set), tool rows default collapsed (open = in-set).
 *  Single source for the default: the bulk-toggle write ends and the detail
 *  view's read end (isRowOpen) all hang off it, so a new default-collapsed
 *  role changes one line here, not two files. */
export function roleDefaultsCollapsed(role: string): boolean {
  return role === "tool"
}

/** A row's open state: in-set xor defaults-to-collapsed. The detail view's
 *  per-row isOpen — lifted here so the read side and the bulk-toggle write
 *  side share the one xor rule. */
export function isRowOpen(
  uuid: string,
  role: string,
  collapsed: ReadonlySet<string>,
): boolean {
  return roleDefaultsCollapsed(role)
    ? collapsed.has(uuid)
    : !collapsed.has(uuid)
}

/**
 * The collapsed-row sets for the bulk collapse / expand toggle. Row open-state
 * is a Set<uuid> whose membership means the OPPOSITE of the row's default (see
 * roleDefaultsCollapsed). So "collapse all" = every default-expanded uuid in
 * the set, and "expand all" = every default-collapsed uuid in the set
 * (messages drop out, tools join). Pure so the toggle's end states are
 * testable — see isAllCollapsed.
 */
export function collapseAllMessages(
  messages: readonly SessionMessage[],
): Set<string> {
  const out = new Set<string>()
  for (const m of messages) {
    if (!roleDefaultsCollapsed(m.role)) out.add(m.uuid)
  }
  return out
}

export function expandAllMessages(
  messages: readonly SessionMessage[],
): Set<string> {
  const out = new Set<string>()
  for (const m of messages) {
    if (roleDefaultsCollapsed(m.role)) out.add(m.uuid)
  }
  return out
}

/** Is every message row collapsed — the "collapse all" end state? False when
 *  the transcript has no expandable (default-expanded) rows (e.g. tool-only or
 *  empty), so the toggle never reports a full-collapse on a transcript that
 *  has no messages to collapse. */
export function isAllCollapsed(
  messages: readonly SessionMessage[],
  collapsed: ReadonlySet<string>,
): boolean {
  let sawExpandable = false
  for (const m of messages) {
    if (roleDefaultsCollapsed(m.role)) continue
    sawExpandable = true
    if (!collapsed.has(m.uuid)) return false
  }
  return sawExpandable
}

// ---------------------------------------------------- transcript-wide search --

/** A transcript-search hit: the matching message plus a short context window
 *  around its first hit, for the result-list row. */
export interface TranscriptMatch {
  message: SessionMessage
  snippet: string
}

/** Search every message body for `query` (case-insensitive substring). The
 *  transcript is fully loaded in the detail sheet, so this is a local scan —
 *  no backend round-trip. Hits come back in transcript order, each with a
 *  snippet that keeps the first hit in view. Empty / whitespace query → no
 *  hits. */
export function transcriptMatches(
  messages: readonly SessionMessage[],
  query: string,
): TranscriptMatch[] {
  const q = query.trim().toLowerCase()
  if (!q) return []
  const out: TranscriptMatch[] = []
  for (const m of messages) {
    const idx = m.content.toLowerCase().indexOf(q)
    if (idx === -1) continue
    out.push({ message: m, snippet: snippetAround(m.content, idx, q.length) })
  }
  return out
}

/** A compact window around a hit: RADIUS chars either side, ellipsized at the
 *  edges. The hit itself stays intact inside the window so the renderer can
 *  highlight it. */
function snippetAround(text: string, start: number, len: number): string {
  const RADIUS = 28
  const from = Math.max(0, start - RADIUS)
  const to = Math.min(text.length, start + len + RADIUS)
  return `${from > 0 ? "…" : ""}${text.slice(from, to)}${
    to < text.length ? "…" : ""
  }`
}
