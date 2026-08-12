// Keyword highlighting for the sessions views — shared by the table's title
// (search-box matches) and the detail sheet's in-session search results.
// Single source of truth: a match is rendered the same way everywhere.

import type { ReactNode } from "react"

/**
 * Mark the query's occurrences in `text` with a highlight — case-insensitive,
 * every hit wrapped in <mark>. Returns plain `text` when there's nothing to
 * highlight (empty query / no hit), so callers can treat it as text-or-nodes.
 * Pure display — produces JSX, so it lives in a .tsx module, not derive.ts.
 */
export function highlight(text: string, query: string): ReactNode {
  const q = query.trim()
  if (!q) return text
  const lower = text.toLowerCase()
  const needle = q.toLowerCase()
  const parts: ReactNode[] = []
  let i = 0
  for (
    let idx = lower.indexOf(needle);
    idx !== -1;
    idx = lower.indexOf(needle, i)
  ) {
    if (idx > i) parts.push(text.slice(i, idx))
    parts.push(
      <mark
        key={idx}
        className="bg-accent-tint text-accent-brand-strong rounded-[3px] px-0.5"
      >
        {text.slice(idx, idx + q.length)}
      </mark>,
    )
    i = idx + q.length
  }
  if (i < text.length) parts.push(text.slice(i))
  return parts.length > 0 ? parts : text
}
