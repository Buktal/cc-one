// Pure derivations for the sessions browser: tab → backend filter, sort order,
// substring search, and the two-track grouping (local / synced) that drives the
// sidebar buckets. Extracted from the hook so each rule is testable in
// isolation (architecture.md: "关键不变量用代码表达") — the hook wires these to
// React state + RTK Query, these own the math.

import type {
  SessionFilter,
  SessionGroup,
  SessionRow,
} from "@/types/generated/bindings"

/**
 * The two top-level tabs (ADR 0002). Local = this device's collected sessions
 * (favorited ones still listed, star lit). Favorites = every favorited session
 * across all devices.
 */
export type SessionTab = "local" | "favorites"

/**
 * The two grouping tracks. The Local tab groups by `local_group_id`
 * (device-private, never enters git); the Favorites tab groups by
 * `synced_group_id` (per-device groups.json, git-synced). A session may sit in
 * different groups across the two tracks — two independent organizations.
 */
export type GroupTrack = "local" | "synced"

/**
 * Sidebar selection sentinels. `ALL_GROUPS` renders every session in the tab;
 * `UNGROUPED` renders only those without a group in this track; a real group id
 * filters to that one bucket. Lives here so the view and hook share one definition.
 */
export const ALL_GROUPS = "__all__"
export const UNGROUPED = "__ungrouped__"

/** Result of grouping a flat session list by one track. */
export interface GroupedSessions {
  /** One entry per group that has at least one session, in group-list order. */
  groups: { group: SessionGroup; sessions: SessionRow[] }[]
  /** Sessions whose track group id is empty or points to a missing group. */
  ungrouped: SessionRow[]
}

/**
 * Extra filter dimensions the sessions toolbar exposes on top of the tab. All
 * optional — omitted/empty values mean "no constraint". Mirrors the logs view's
 * toolbar (time range · source · model · device) so the two data views filter
 * the same way. Model is EXISTS semantics (a session that used the model at
 * least once matches) — the model lives per-request on usage_records, never on
 * the session row.
 */
export interface SessionListFilter {
  /** Source tag, e.g. "claude_code". */
  source?: string | null
  /** Inclusive lower bound on `last_active_at` (ISO8601). */
  fromTs?: string | null
  /** Inclusive upper bound on `last_active_at` (ISO8601). */
  toTs?: string | null
  /**
   * Device id to narrow the Favorites tab ("all devices" by default). Ignored
   * on the Local tab — that tab is always this device.
   */
  deviceScope?: string | null
  /** Session used this model at least once (EXISTS over usage_records). */
  model?: string | null
}

/**
 * Build the backend `SessionFilter` for a tab. Local → only this device's
 * sessions (favorited or not); Favorites → only favorited, across all devices.
 * `null` fields mean "no constraint" (see SessionFilter docs). Group fields stay
 * `null` — grouping is a client-side render concern (groupSessionsByGroup), so
 * the backend returns the whole tab slice once and the sidebar filters locally.
 *
 * `filter.source` / `filter.fromTs` / `filter.toTs` narrow the tab slice and are
 * applied backend-side (single source of truth — never re-applied client-side).
 * `filter.deviceScope` narrows the Favorites tab to one device; on the Local
 * tab it is ignored (always this device). The substring search box is a separate
 * client-side concern (filterSessionsByQuery).
 */
export function sessionTabFilter(
  tab: SessionTab,
  selfDeviceId: string,
  filter: SessionListFilter = {},
): SessionFilter {
  const src = filter.source || null
  const fromTs = filter.fromTs || null
  const toTs = filter.toTs || null
  const model = filter.model || null
  if (tab === "local") {
    return {
      device_scope: selfDeviceId,
      source: src,
      favorited: null,
      local_group_id: null,
      synced_group_id: null,
      from_ts: fromTs,
      to_ts: toTs,
      model,
    }
  }
  // Favorites tab: deviceScope narrows from "all devices" to one.
  return {
    device_scope: filter.deviceScope || null,
    source: src,
    favorited: true,
    local_group_id: null,
    synced_group_id: null,
    from_ts: fromTs,
    to_ts: toTs,
    model,
  }
}

/**
 * Sort sessions by `last_active_at` descending (most recent first). Missing or
 * unparseable timestamps sort last. Returns a new array — input is not mutated.
 */
export function sortSessions(rows: SessionRow[]): SessionRow[] {
  return [...rows].sort(
    (a, b) => toMillis(b.last_active_at) - toMillis(a.last_active_at),
  )
}

function toMillis(ts: string | null | undefined): number {
  if (!ts) return 0
  const n = Date.parse(ts)
  return Number.isNaN(n) ? 0 : n
}

/**
 * Case-insensitive substring filter over title and project path. An empty or
 * whitespace-only query returns the input unchanged (no constraint) so callers
 * can pipe through unconditionally.
 */
export function filterSessionsByQuery(
  rows: SessionRow[],
  q: string,
): SessionRow[] {
  const needle = q.trim().toLowerCase()
  if (!needle) return rows
  return rows.filter((r) => {
    const title = (r.title ?? "").toLowerCase()
    const project = (r.project_dir ?? "").toLowerCase()
    return title.includes(needle) || project.includes(needle)
  })
}

/**
 * Bucket sessions by one grouping track. A session whose `local_group_id`
 * (local track) or `synced_group_id` (synced track) is empty OR references a
 * group id not present in `groups` falls into `ungrouped` — a stale id left by
 * a deleted group is treated as ungrouped, never silently dropped. Input order
 * is preserved within each bucket — pass pre-sorted rows to get sorted buckets.
 * Groups with zero sessions are omitted here (empty groups still appear in the
 * sidebar, sourced from the raw group list).
 */
export function groupSessionsByGroup(
  rows: SessionRow[],
  groups: SessionGroup[],
  track: GroupTrack,
): GroupedSessions {
  const trackGroups = groups.filter((g) => g.kind === track)
  const knownIds = new Set(trackGroups.map((g) => g.id))
  const buckets = new Map<string, SessionRow[]>()
  const ungrouped: SessionRow[] = []
  for (const row of rows) {
    const gid = track === "local" ? row.local_group_id : row.synced_group_id
    if (gid && knownIds.has(gid)) {
      const arr = buckets.get(gid)
      if (arr) arr.push(row)
      else buckets.set(gid, [row])
    } else {
      ungrouped.push(row)
    }
  }
  return {
    groups: trackGroups
      .filter((g) => buckets.has(g.id))
      .map((g) => ({ group: g, sessions: buckets.get(g.id) ?? [] })),
    ungrouped,
  }
}

/**
 * Resolve the sidebar selection to the sessions to render. `ALL_GROUPS` → every
 * session in the tab (pass the full sorted+filtered list as `allRows` so the
 * flat order is preserved, not the grouped concatenation); `UNGROUPED` → only
 * the ungrouped bucket; a real group id → that bucket (empty if unknown).
 */
export function selectSessions(
  allRows: SessionRow[],
  grouped: GroupedSessions,
  selectedGroupId: string,
): SessionRow[] {
  if (selectedGroupId === ALL_GROUPS) return allRows
  if (selectedGroupId === UNGROUPED) return grouped.ungrouped
  return (
    grouped.groups.find((g) => g.group.id === selectedGroupId)?.sessions ?? []
  )
}

// --------------------------------------------------------------- favorites --

/**
 * Composite identity key for a session — `device_id/id`. A session is uniquely
 * (device_id, id): the same id can exist on two devices. Shared by the favorite
 * override map and the preview lookup so both resolve the same row.
 */
export function favKey(s: { device_id: string; id: string }): string {
  return `${s.device_id}/${s.id}`
}

/**
 * A session's effective favorite: a pending optimistic override wins over the
 * query value, falling back to the row's `favorited` when no override exists.
 * The override map is keyed by `favKey`.
 */
export function effectiveFavorite(
  s: SessionRow,
  overrides: Record<string, boolean>,
): boolean {
  const k = favKey(s)
  return k in overrides ? overrides[k] : s.favorited
}

/**
 * The value to optimistically stamp (and send to the mutation) for a favorite
 * toggle: the negation of the effective state.
 */
export function nextFavValue(
  s: SessionRow,
  overrides: Record<string, boolean>,
): boolean {
  return !effectiveFavorite(s, overrides)
}

/**
 * Stamp an optimistic favorite override. Returns a NEW map — never mutates the
 * input (React state updates must be pure).
 */
export function withFavOverride(
  overrides: Record<string, boolean>,
  s: { device_id: string; id: string },
  value: boolean,
): Record<string, boolean> {
  return { ...overrides, [favKey(s)]: value }
}

/**
 * Drop a favorite override (rollback after a failed toggle). Returns a NEW map
 * without the key — the effective value then falls back to the row's
 * `favorited`.
 */
export function withoutFavOverride(
  overrides: Record<string, boolean>,
  s: { device_id: string; id: string },
): Record<string, boolean> {
  const next = { ...overrides }
  delete next[favKey(s)]
  return next
}

/**
 * May a new group be created on this track? Local groups are always allowed
 * (SQLite, device-private); synced (Favorites-tab) groups live in git
 * (`data/<deviceId>/groups.json`), so they need a bound Git repo — without one
 * the create would silently fail or hang.
 */
export function canCreateSyncedGroup(
  track: GroupTrack,
  synced: boolean,
): boolean {
  return track !== "synced" || synced
}

/**
 * The new track order after a drag — moved to `src/lib/reorder.ts` when the
 * providers list needed the same arrayMove semantics; re-exported here under
 * the sessions name so existing call sites keep working.
 */
export { reorderIds as reorderGroupIds } from "@/lib/reorder"

/**
 * Apply an optimistic drag-override to the track groups: `orderedIds` is the
 * new order the user just dragged to. Groups the override no longer knows
 * (deleted mid-flight) sort to the end in their original relative order —
 * never dropped. null override = natural order.
 */
export function applyGroupOrder(
  groups: SessionGroup[],
  orderedIds: string[] | null,
): SessionGroup[] {
  if (!orderedIds) return groups
  const rank = new Map(orderedIds.map((id, i) => [id, i]))
  return [...groups].sort(
    (a, b) => (rank.get(a.id) ?? 1_000_000) - (rank.get(b.id) ?? 1_000_000),
  )
}

// ------------------------------------------------------------- transcript --

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
