// Pure derivations for the sessions browser: tab → backend filter, sort order,
// substring search, and the two-track grouping (local / synced) that drives the
// sidebar buckets. Extracted from the hook so each rule is testable in
// isolation (architecture.md: "关键不变量用代码表达") — the hook wires these to
// React state + RTK Query, these own the math.

import {
  FILTER_DIMENSIONS,
  type FilterState,
} from "@/app/store/slices/filterSlice"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import { ALL_FILTER } from "@/lib/source-tags"
import type {
  SessionFilter,
  SessionGroup,
  SessionGroupCounts,
  SessionMessage,
  SessionRow,
  SessionStatsRow,
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
export const ALL_GROUPS = ALL_FILTER
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
  /**
   * Project identity to narrow to (matched backend-side through the
   * `project_identity` rule, so a worktree session counts under its parent
   * project). No UI feeds this yet — the project tree track is a later ticket;
   * null = "no constraint".
   */
  project?: string | null
  /**
   * Substring search over the display title, the project path, and every
   * message body (backend LIKE — bodies are probed across all devices of the
   * session, the same union the merged transcript shows).
   */
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
  const project = filter.project || null
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
      project,
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
    project,
    from_ts: fromTs,
    to_ts: toTs,
    model,
    search,
  }
}

/**
 * The left tree's three tracks (定稿 docs/plans/sessions-workbench-redesign.md
 * §1): 项目 = automatic project buckets, 分组 = manual local groups, 收藏 =
 * synced groups over the cross-device favorites universe. The track replaces
 * the old Local / Favorites tabs as the page's universe switch.
 */
export type TreeTrack = "projects" | "groups" | "favorites"

/** The session universe a tree track reads: the project / group tracks show
 *  this device's sessions (the old Local tab), the favorites track shows
 *  favorited sessions across devices (the old Favorites tab). */
export function trackUniverseTab(track: TreeTrack): SessionTab {
  return track === "favorites" ? "favorites" : "local"
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
  /** Selected project identity (the tree's project track / the narrow-window
   *  project dropdown); null = no project constraint. */
  project: string | null
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
      project: spec.project,
      search: spec.search,
    },
    spec.selectedGroupId,
  )
}

/**
 * Stable cache id for a sessions scope (mirrors filterId on the usage side):
 * built from the logical dimensions only, so a dynamic preset stays stable
 * across a day and the bounds roll via the refresh chain. Concatenates the
 * scope's own dimensions plus every FilterState dimension (FILTER_DIMENSIONS)
 * — derive.test.ts fails if one is missed, so differing dimension values can
 * never silently share a cache entry.
 */
export function sessionSpecId(spec: SessionScopeSpec): string {
  return [
    spec.tab,
    spec.selfDeviceId,
    spec.selectedGroupId,
    spec.project ?? "",
    spec.search ?? "",
    ...FILTER_DIMENSIONS.map((k) => spec.filter[k]),
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

/** 会话时长的展示文案选择（架构扫描候选⑨c 顺带）：把「有天数 → 天+小时；
 *  有小时 → 小时+分钟（有分钟时）/ 纯小时；否则纯分钟；无时长 → null」的
 *  四层嵌套三元收成可测纯函数——只选键与插值变量，文案翻译仍由调用方
 *  `t()` 完成。null = 无可用时长，调用方渲染占位符（—）。 */
export function spanLabelKey(span: SessionSpan | null): {
  key:
    | "sessions.span.daysHours"
    | "sessions.span.hoursMinutes"
    | "sessions.span.hours"
    | "sessions.span.minutes"
  vars: Record<string, number>
} | null {
  if (!span) return null
  if (span.days > 0) {
    return {
      key: "sessions.span.daysHours",
      vars: { d: span.days, h: span.hours },
    }
  }
  if (span.hours > 0) {
    return span.minutes > 0
      ? {
          key: "sessions.span.hoursMinutes",
          vars: { h: span.hours, m: span.minutes },
        }
      : { key: "sessions.span.hours", vars: { h: span.hours } }
  }
  return { key: "sessions.span.minutes", vars: { m: span.minutes } }
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

// ------------------------------------------------------- workbench stats ----

/**
 * The four token buckets the workbench aggregates over — the JS mirror of the
 * Rust `TokenCounts` u32 four-pack the backend rows carry.
 */
export interface StatsTokens {
  input: number
  output: number
  cache_creation: number
  cache_read: number
}

/**
 * Cache-hit ratio over the cacheable pool: cache_read / (input +
 * cache_creation + cache_read). Mirrors the Rust `TokenCounts::cache_hit_rate`
 * (the single implementation the backend rows read); kept here because the
 * workbench aggregates rows client-side and a ratio is not additive — only
 * the summed buckets can feed it. null when the pool is empty (no usage).
 */
export function tokensHitRate(t: StatsTokens): number | null {
  const pool = t.input + t.cache_creation + t.cache_read
  return pool > 0 ? t.cache_read / pool : null
}

/** A session's span in ms (last_active − started); null when absent or not
 *  positive — the duration buckets skip those instead of counting garbage. */
export function statsSpanMs(
  s: Pick<SessionStatsRow, "started_at" | "last_active_at">,
): number | null {
  if (!s.started_at || !s.last_active_at) return null
  const ms = Date.parse(s.last_active_at) - Date.parse(s.started_at)
  return Number.isFinite(ms) && ms > 0 ? ms : null
}

/** One per-model share of an aggregate — tokens are the model's four-bucket
 *  sum, `sessions` how many rows used it (the card's sub-line). */
export interface ModelShare {
  model: string
  tokens: number
  sessions: number
}

/** The workbench's right-rail aggregate over a set of session stats rows:
 *  additive sums (buckets / requests / messages / cost), the derived hit rate
 *  (NOT the mean of row rates — see tokensHitRate), per-model token shares,
 *  and the duration-bucket counts (<15m / 15–60m / 1–3h / >3h). Pure so the
 *  口径 invariants are testable. */
export interface StatsAggregate {
  sessions: number
  requests: number
  messages: number
  tokens: StatsTokens
  hitRate: number | null
  cost: number
  models: ModelShare[]
  durationBuckets: [number, number, number, number]
  /** Sum of the VALID session spans (ms) — the「累计时长」figure; null when
   *  no row had a usable span (the caller renders a dash). */
  totalSpanMs: number | null
  lastActiveAt: string | null
}

export function aggregateStats(
  rows: readonly SessionStatsRow[],
): StatsAggregate {
  const tokens = { input: 0, output: 0, cache_creation: 0, cache_read: 0 }
  const byModel = new Map<string, ModelShare>()
  const durationBuckets: [number, number, number, number] = [0, 0, 0, 0]
  let requests = 0
  let messages = 0
  let cost = 0
  let totalSpanMs = 0
  let sawSpan = false
  let last: string | null = null
  for (const r of rows) {
    requests += r.request_count
    messages += r.message_count
    tokens.input += r.input_tokens
    tokens.output += r.output_tokens
    tokens.cache_creation += r.cache_creation_tokens
    tokens.cache_read += r.cache_read_tokens
    cost += r.total_cost_usd ?? 0
    // Rows arrive last-active-desc from the backend, but a filtered slice may
    // come from anywhere — compare, don't trust the order.
    if (r.last_active_at && (!last || r.last_active_at > last))
      last = r.last_active_at
    for (const m of r.models) {
      const slot = byModel.get(m.model) ?? {
        model: m.model,
        tokens: 0,
        sessions: 0,
      }
      slot.tokens += m.tokens
      slot.sessions += 1
      byModel.set(m.model, slot)
    }
    const span = statsSpanMs(r)
    if (span !== null) {
      sawSpan = true
      totalSpanMs += span
      const minutes = span / 60_000
      durationBuckets[
        minutes < 15 ? 0 : minutes < 60 ? 1 : minutes < 180 ? 2 : 3
      ] += 1
    }
  }
  return {
    sessions: rows.length,
    requests,
    messages,
    tokens,
    hitRate: tokensHitRate(tokens),
    cost,
    models: [...byModel.values()].sort((a, b) => b.tokens - a.tokens),
    durationBuckets,
    totalSpanMs: sawSpan ? totalSpanMs : null,
    lastActiveAt: last,
  }
}

/** The project track's buckets: sessions grouped by project identity, each
 *  bucket carrying its row slice (backend order — last-active desc), its
 *  token total, and its newest last-active for the bucket ordering. Empty
 *  project dirs bucket under "" (rendered as「无项目」). */
export interface ProjectNodeData {
  project: string
  sessions: SessionStatsRow[]
  tokens: number
  lastActiveAt: string
}

export function projectNodes(
  rows: readonly SessionStatsRow[],
): ProjectNodeData[] {
  const byProject = new Map<string, ProjectNodeData>()
  for (const r of rows) {
    const node = byProject.get(r.project_dir) ?? {
      project: r.project_dir,
      sessions: [],
      tokens: 0,
      lastActiveAt: r.last_active_at,
    }
    node.sessions.push(r)
    node.tokens +=
      r.input_tokens +
      r.output_tokens +
      r.cache_creation_tokens +
      r.cache_read_tokens
    if (r.last_active_at > node.lastActiveAt) node.lastActiveAt = r.last_active_at
    byProject.set(r.project_dir, node)
  }
  return [...byProject.values()].sort((a, b) =>
    b.lastActiveAt.localeCompare(a.lastActiveAt),
  )
}

/**
 * The group tracks' row bucketing: rows keyed by their group id, with every
 * row whose id is empty OR not in `knownIds` falling into `ungrouped` — the
 * same "stale id counts as ungrouped" rule `ungroupedCount` applies to the
 * backend counts, kept in one place for the tree's session children.
 */
export function groupedRows<T>(
  rows: readonly T[],
  groupIdOf: (row: T) => string,
  knownIds: ReadonlySet<string>,
): { grouped: Map<string, T[]>; ungrouped: T[] } {
  const grouped = new Map<string, T[]>()
  const ungrouped: T[] = []
  for (const r of rows) {
    const id = groupIdOf(r)
    if (!id || !knownIds.has(id)) ungrouped.push(r)
    else {
      const bucket = grouped.get(id)
      if (bucket) bucket.push(r)
      else grouped.set(id, [r])
    }
  }
  return { grouped, ungrouped }
}

/** Display basename of a project dir — the tree / cards / tables show the
 *  short name with the full path on hover (定稿 §痛点1). Handles both path
 *  separators; a path that is all separators (or empty) has no basename and
 *  renders as-is (the caller decides the「无项目」placeholder). */
export function projectBasename(dir: string): string {
  const trimmed = dir.replace(/[\\/]+$/, "")
  const slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
  return slash === -1 ? trimmed : trimmed.slice(slash + 1)
}

/** Project a stats row onto the list-row shape — the tree's session children
 *  are stats rows, but the detail view consumes SessionRow. The numbers come
 *  from the same SQL aggregates the list reads (buckets summed = the list's
 *  total_tokens; cost identical), so the projection is lossless for every
 *  field the detail uses. */
export function toSessionRow(r: SessionStatsRow): SessionRow {
  return {
    id: r.id,
    device_id: r.device_id,
    source: r.source,
    project_dir: r.project_dir,
    title: r.title,
    agent_type: r.agent_type,
    favorited: r.favorited,
    local_group_id: r.local_group_id,
    synced_group_id: r.synced_group_id,
    started_at: r.started_at,
    last_active_at: r.last_active_at,
    request_count: r.request_count,
    total_tokens:
      r.input_tokens +
      r.output_tokens +
      r.cache_creation_tokens +
      r.cache_read_tokens,
    total_cost_usd: r.total_cost_usd,
  }
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

// ------------------------------------------------------------ search -----

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
