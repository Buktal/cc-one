// Pure derivations for the sessions browser: tab → backend filter, sort order,
// substring search, and the two-track grouping (local / synced) that drives the
// sidebar buckets. Extracted from the hook so each rule is testable in
// isolation (architecture.md: "关键不变量用代码表达") — the hook wires these to
// React state + RTK Query, these own the math.

import type { FilterState } from "@/app/store/slices/filterSlice"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import type {
  SessionFilter,
  SessionGroup,
  SessionGroupCounts,
  SessionMessage,
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

/**
 * Extra filter dimensions the sessions toolbar exposes on top of the tab. All
 * optional — omitted/empty values mean "no constraint". Mirrors the logs view's
 * toolbar (time range · source · model · device) so the two data views filter
 * the same way. Model is EXISTS semantics (a session that used the model at
 * least once matches) — the model lives per-request on usage_records, never on
 * the session row. Search is backend-side too (paged results would otherwise
 * only search the loaded page) — see sessionTabFilter.
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
  /** Substring search over the display title and project path (backend LIKE). */
  search?: string | null
}

/**
 * Build the backend `SessionFilter` for a tab. Local → only this device's
 * sessions (favorited or not); Favorites → only favorited, across all devices.
 * `null` fields mean "no constraint" (see SessionFilter docs).
 *
 * `filter.source` / `filter.fromTs` / `filter.toTs` / `filter.model` /
 * `filter.search` narrow the tab slice and are applied backend-side (single
 * source of truth — never re-applied client-side). `filter.deviceScope`
 * narrows the Favorites tab to one device; on the Local tab it is ignored
 * (always this device).
 *
 * `groupId` encodes the sidebar selection as a backend filter on the track's
 * group column (local tab → `local_group_id`, favorites → `synced_group_id`):
 * `null` = "All groups" (no constraint), `UNGROUPED` = empty string (matches
 * ungrouped), any other id narrows to that group. Grouping is therefore also a
 * backend concern — the page query returns exactly the selected bucket, so a
 * large group pages like the All view instead of loading every row.
 */
export function sessionTabFilter(
  tab: SessionTab,
  selfDeviceId: string,
  filter: SessionListFilter = {},
  groupId: string | null = null,
): SessionFilter {
  const src = filter.source || null
  const fromTs = filter.fromTs || null
  const toTs = filter.toTs || null
  const model = filter.model || null
  const search = filter.search || null
  const trackGroup = (): string | null => {
    if (groupId == null || groupId === ALL_GROUPS) return null
    return groupId === UNGROUPED ? "" : groupId
  }
  if (tab === "local") {
    return {
      device_scope: selfDeviceId,
      source: src,
      favorited: null,
      local_group_id: trackGroup(),
      synced_group_id: null,
      from_ts: fromTs,
      to_ts: toTs,
      model,
      search,
    }
  }
  // Favorites tab: deviceScope narrows from "all devices" to one.
  return {
    device_scope: filter.deviceScope || null,
    source: src,
    favorited: true,
    local_group_id: null,
    synced_group_id: trackGroup(),
    from_ts: fromTs,
    to_ts: toTs,
    model,
    search,
  }
}

/**
 * A sessions query scope: the common dimensions (from filterSlice) + the
 * sessions-only dimensions (tab / group / search) + the self device id a
 * SessionFilter needs. Carries NO timestamp — bounds are derived in
 * buildSessionFilter at query time, so the cache key (sessionSpecId) stays
 * stable across a day.
 */
export interface SessionScopeSpec {
  /** Common dimensions shared with the dashboard / logs (time / model / source / device). */
  filter: FilterState
  /** Sessions-only: which tab, which sidebar group, and the search box. */
  tab: SessionTab
  selfDeviceId: string
  selectedGroupId: string
  search: string | null
}

/**
 * Build the backend SessionFilter from a scope, deriving the timestamp bounds
 * from the current date. Shared by the listSessions + sessionCounts
 * queryFns so both reads see identical bounds. Bounds are a query-time concern
 * here — never stored or displayed, so the scope (and its cache key) carries
 * no timestamp.
 */
export function buildSessionFilter(spec: SessionScopeSpec): SessionFilter {
  const { from_day, to_day } = effectiveDays(spec.filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  return sessionTabFilter(
    spec.tab,
    spec.selfDeviceId,
    {
      source: spec.filter.source || null,
      fromTs,
      toTs,
      deviceScope: spec.filter.device_scope || null,
      model: spec.filter.model || null,
      search: spec.search,
    },
    spec.selectedGroupId,
  )
}

/**
 * Stable cache id for a sessions scope (mirrors filterId on the usage side):
 * built from the logical dimensions only, so a dynamic preset stays stable
 * across a day and the bounds roll via the refresh chain.
 */
export function sessionSpecId(spec: SessionScopeSpec): string {
  return [
    spec.tab,
    spec.selfDeviceId,
    spec.selectedGroupId,
    spec.search ?? "",
    spec.filter.range_preset,
    spec.filter.from_day,
    spec.filter.to_day,
    spec.filter.model,
    spec.filter.source,
    spec.filter.device_scope,
  ].join("|")
}

// ---------------------------------------------------------------- counts ----

/**
 * The sidebar's ungrouped count from the backend's per-bucket counts: total
 * minus the buckets belonging to known groups. Every other bucket — the empty
 * id (ungrouped) and stale ids whose group was deleted — counts as ungrouped,
 * the same rule the old client-side grouping applied (a stale id is treated as
 * ungrouped, never silently dropped). Pure so the invariant is testable.
 */
export function ungroupedCount(
  counts: Pick<SessionGroupCounts, "total" | "groups">,
  knownGroupIds: ReadonlySet<string>,
): number {
  let known = 0
  for (const g of counts.groups) {
    if (knownGroupIds.has(g.group_id)) known += g.count
  }
  return counts.total - known
}

// -------------------------------------------------------------- detail -----

/**
 * A session's elapsed span (`last_active_at − started_at`), split into the
 * units the detail header displays. `null` when the times are absent /
 * unparseable / the span is not positive — the header then shows a dash
 * instead of a bogus negative duration.
 */
export interface SessionSpan {
  days: number
  hours: number
  minutes: number
}

export function sessionSpan(ms: number | null | undefined): SessionSpan | null {
  const v = Number(ms ?? 0)
  if (!Number.isFinite(v) || v <= 0) return null
  const totalMinutes = Math.floor(v / 60_000)
  return {
    days: Math.floor(totalMinutes / (24 * 60)),
    hours: Math.floor((totalMinutes % (24 * 60)) / 60),
    minutes: totalMinutes % 60,
  }
}

/**
 * Distinct model names used in a transcript, in first-use order (assistant
 * messages carry the model; user/tool/system rows have none). Empty input ⇒
 * empty list — the caller decides how to render "no models yet".
 */
export function modelsUsed(messages: readonly SessionMessage[]): string[] {
  const seen = new Set<string>()
  const out: string[] = []
  for (const m of messages) {
    const model = m.model?.trim()
    if (model && !seen.has(model)) {
      seen.add(model)
      out.push(model)
    }
  }
  return out
}

/**
 * Whether the detail sheet's prev / next navigation can move from the row at
 * `previewKey` within the visible page: an adjacent row exists (idx ± 1), or
 * the row sits at a page edge and another page exists to page into. A preview
 * key not on the page (the filter / tab changed mid-session) disables both
 * directions — navigation only walks the currently visible list.
 */
export function neighborNav(
  rows: readonly SessionRow[],
  previewKey: string | null,
  offset: number,
  pageSize: number,
  totalCount: number,
): { canPrev: boolean; canNext: boolean } {
  if (!previewKey) return { canPrev: false, canNext: false }
  const idx = rows.findIndex((s) => favKey(s) === previewKey)
  if (idx === -1) return { canPrev: false, canNext: false }
  return {
    canPrev: idx > 0 || offset > 0,
    canNext: idx < rows.length - 1 || offset + pageSize < totalCount,
  }
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

/**
 * The collapsed-row sets for the bulk collapse / expand toggle. Row open-state
 * is a Set<uuid> whose membership means the OPPOSITE of the row's default:
 * messages default expanded (open = not-in-set), tool rows default collapsed
 * (open = in-set). So "collapse all" = every message uuid in the set, and
 * "expand all" = every tool uuid in the set (messages drop out, tools join).
 * Pure so the toggle's end states are testable — see isAllCollapsed.
 */
export function collapseAllMessages(
  messages: readonly SessionMessage[],
): Set<string> {
  const out = new Set<string>()
  for (const m of messages) {
    if (m.role !== "tool") out.add(m.uuid)
  }
  return out
}

export function expandAllMessages(
  messages: readonly SessionMessage[],
): Set<string> {
  const out = new Set<string>()
  for (const m of messages) {
    if (m.role === "tool") out.add(m.uuid)
  }
  return out
}

/** Is every message row collapsed — the "collapse all" end state? False when
 *  the transcript has no expandable (non-tool) rows (e.g. tool-only or empty),
 *  so the toggle never reports a full-collapse on a transcript that has no
 *  messages to collapse. */
export function isAllCollapsed(
  messages: readonly SessionMessage[],
  collapsed: ReadonlySet<string>,
): boolean {
  let sawExpandable = false
  for (const m of messages) {
    if (m.role === "tool") continue
    sawExpandable = true
    if (!collapsed.has(m.uuid)) return false
  }
  return sawExpandable
}
