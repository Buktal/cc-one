// Pure derivations for the sessions browser — the LIST surface only: the
// tab/track → backend filter assembly and its cache key, the container-
// selection union (which bucket the workbench is looking at), the workbench's
// stats aggregates, the favorites' optimistic-override math, and the detail
// sheet's prev/next neighbor stepping (canPrev/canNext plus the page-edge
// step plan and its settlement). Extracted from the hook so each rule is
// testable in isolation (architecture.md: "关键不变量用代码表达") — the hook
// wires these to React state + RTK Query, these own the math. The transcript
// detail view has its own modules (./transcript, turn-nav.ts, turn-search.ts,
// conversation.ts) and does not read from here.

import {
  FILTER_DIMENSIONS,
  type FilterState,
} from "@/app/store/slices/filterSlice"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import { spanMsOf } from "@/lib/format"
import { projectBasename } from "@/lib/paths"
import { ALL_FILTER } from "@/lib/source-tags"
import { type TokenBuckets, totalTokensOf } from "@/lib/token-buckets"
import type {
  GroupTrack,
  SessionFilter,
  SessionGroup,
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
 * Re-exported from the generated bindings (the backend `GroupTrack` enum's TS
 * type) so the union has one definition, not a hand-copied twin.
 */
export type { GroupTrack }

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
 * toolbar (time range · source · model · device · project) so the two data
 * views filter the same way. Model is EXISTS semantics (a session that used the
 * model at least once matches) — the model lives per-request on usage_records,
 * never on the session row. Search is backend-side too (paged results would
 * otherwise only search the loaded page) — see sessionTabFilter.
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
   * project). Fed from the SHARED filterSlice project dimension (the same
   * constraint the dashboard / request-log project dropdowns write) — the
   * sessions tree's project track and the toolbar dropdown are two surfaces of
   * that one state. The unknown sentinel (arriving as endpoint data) is a
   * value like any other; null = "no constraint".
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
 * A sessions query scope: the common dimensions (from filterSlice — including
 * `project`, shared with the dashboard / logs dropdowns and the tree's project
 * track) + the sessions-only dimensions (tab / group / search) + the self
 * device id a SessionFilter needs. Carries NO timestamp — bounds are derived
 * in buildSessionFilter at query time, so the cache key (sessionSpecId) stays
 * stable across a day.
 */
export interface SessionScopeSpec {
  /** Common dimensions shared with the dashboard / logs (time / model / source
   *  / device / project). */
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
 * no timestamp. The project constraint rides `spec.filter.project` (the shared
 * dimension; the sentinel value, when present, keeps its backend semantics —
 * sessions with an empty project identity).
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
      project: spec.filter.project || null,
      search: spec.search,
    },
    spec.selectedGroupId,
  )
}

/**
 * Stable cache id for a sessions scope (mirrors filterId on the usage side):
 * built from the logical dimensions only, so a dynamic preset stays stable
 * across a day and the bounds roll via the refresh chain. Concatenates the
 * scope's own dimensions plus every FilterState dimension (FILTER_DIMENSIONS,
 * project included) — derive.test.ts fails if one is missed, so differing
 * dimension values can never silently share a cache entry.
 */
export function sessionSpecId(spec: SessionScopeSpec): string {
  return [
    spec.tab,
    spec.selfDeviceId,
    spec.selectedGroupId,
    spec.search ?? "",
    ...FILTER_DIMENSIONS.map((k) => spec.filter[k]),
  ].join("|")
}

// --------------------------------------------- project dimension mapping ----

/**
 * Tree project-bucket identity → filter value. The tree keys buckets by the
 * session-side identity space, where "" is the no-launch-dir bucket (「未知项
 * 目」的会话面); the filter value space uses "" for "no constraint", so the
 * empty bucket can only be expressed through the unknown sentinel — a value
 * the frontend holds as ENDPOINT DATA, never as a literal. When the sentinel
 * is not currently known (`unknownValue` null — no unknown usage has been seen
 * in any window this mount), the empty bucket is not expressible and the
 * click degrades to "no constraint" instead of silently narrowing to nothing.
 */
export function projectFilterOfIdentity(
  identity: string,
  unknownValue: string | null,
): string {
  return identity || unknownValue || ""
}

/**
 * Filter value → tree project-bucket identity (the inverse of
 * projectFilterOfIdentity): "" → null (no selection), the sentinel → "" (the
 * no-launch-dir bucket), any other value → itself (a stale known project that
 * left the window still names itself).
 */
export function identityOfProjectFilter(
  value: string,
  unknownValue: string | null,
): string | null {
  if (!value) return null
  if (unknownValue != null && value === unknownValue) return ""
  return value
}

// -------------------------------------------------------------- detail -----
// 时长三件套（spanParts / spanLabelKey / spanMsOf）已下放 lib/format（架构
// 审查Ⅲ候选⑩）——与「会话」无关的通用时长格式化，usage KPI 等面共用。

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

/**
 * A registered page-edge step: the direction, plus the favKey of the row the
 * user was on when the step was requested. That key is the hijack guard —
 * settleNeighborStep only opens the target while the user still sits on the
 * row the step left from.
 */
export interface PendingNeighborStep {
  delta: 1 | -1
  fromKey: string
}

/** What an "open neighbor" click (detail sheet prev/next) must do —
 *  planNeighborStep's single decision surface. */
export type NeighborStepPlan =
  | { kind: "in-page"; target: SessionRow }
  | { kind: "page-edge"; pending: PendingNeighborStep }
  | { kind: "stalled" }

/**
 * Decide what a neighbor step must do. The three stepping invariants, one per
 * branch:
 *
 * 1. An adjacent row on the visible page opens directly (`in-page`).
 * 2. A page edge with another page beyond it plans a page flip (`page-edge`):
 *    the target row (next page's first / previous page's last) does not exist
 *    until the new page's data lands, so the caller registers `pending` and
 *    shifts pages; settleNeighborStep opens the target on arrival.
 * 3. Nowhere to go → `stalled`. The boundary rule stays neighborNav's single
 *    source (the buttons disable from its canPrev/canNext); planning consults
 *    the same flags, so a step is never planned past the ends even if a
 *    caller bypasses the disabled state.
 *
 * A preview key that is not on the visible page (filter changed mid-session)
 * also stalls — stepping only walks the currently visible list.
 */
export function planNeighborStep(
  rows: readonly SessionRow[],
  previewKey: string | null,
  delta: 1 | -1,
  offset: number,
  pageSize: number,
  totalCount: number,
): NeighborStepPlan {
  const nav = neighborNav(rows, previewKey, offset, pageSize, totalCount)
  if (!previewKey || !(delta === 1 ? nav.canNext : nav.canPrev)) {
    return { kind: "stalled" }
  }
  const idx = rows.findIndex((s) => favKey(s) === previewKey)
  const target = rows[idx + delta]
  if (target) return { kind: "in-page", target }
  return { kind: "page-edge", pending: { delta, fromKey: previewKey } }
}

/** How a pending page-edge step resolves once new list data lands —
 *  settleNeighborStep's single decision surface. */
export type NeighborStepSettlement =
  | { kind: "open"; target: SessionRow }
  | { kind: "drop" }
  | { kind: "wait" }

/**
 * Resolve a pending page-edge step against the freshly landed page data — the
 * "open only after the new page's data arrives" half of the stepping
 * invariants:
 *
 * - The user still sits on the row the step left from (`previewKey ===
 *   pending.fromKey`) and the new page carries rows → `open` its edge row
 *   (next → first row, prev → last row). The caller consumes the pending step.
 * - The user switched to another row while the page loaded (`previewKey` null
 *   or different) → `drop`: the pending step must not hijack their selection.
 * - The flipped-to page has not landed yet (matching key, no rows) → `wait`:
 *   keep the pending step registered for the next data change.
 */
export function settleNeighborStep(
  pending: PendingNeighborStep,
  previewKey: string | null,
  rows: readonly SessionRow[],
): NeighborStepSettlement {
  if (!previewKey || previewKey !== pending.fromKey) return { kind: "drop" }
  const target = pending.delta === 1 ? rows[0] : rows[rows.length - 1]
  if (!target) return { kind: "wait" }
  return { kind: "open", target }
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

// ------------------------------------------ container selection (候选⑤) --

/**
 * 容器选中(container selection)——工作台「当前在看谁」的唯一编码。一个判别
 * 联合、一处构造(resolveContainer):右栏口径行、中栏列表头、窄容器下拉与
 * 统计切片都读它。此前这一概念以五种形状散布在视图与 hook 里(scopeLabel /
 * scopeTag / 树下拉的 p:/g: 编解码 / headTitle / selectionStatsRows),优先级
 * 阶梯只活在注释里——现在阶梯就是 resolveContainer 的分支次序:
 * 会话打开 > 项目 > 分组 > 未分组 > 全部。
 */
export type ContainerSelection =
  | {
      /** 打开的会话:id = favKey 复合键;title 为展示名(空串由渲染方落到
       *  untitled 键);dir 直通行的项目 identity。 */
      kind: "session"
      id: string
      title: string
      dir: string
    }
  /** 项目桶(id 是 identity 空间:"" = 无启动目录桶,哨兵映射后)。 */
  | { kind: "project"; id: string }
  | { kind: "group"; id: string }
  | { kind: "ungrouped" }
  | { kind: "all" }

/**
 * Resolve the container selection. `preview` 非空即会话态——树选中让位但不丢
 * 失(selectProject/selectGroup/selectAll 才会改写它,打开会话不动树);调用
 * 方传 null 得到的是纯树容器视图(树镜像控件用),传真值得到工作台口径对象。
 */
export function resolveContainer(
  preview: Pick<
    SessionRow,
    "id" | "device_id" | "title" | "project_dir"
  > | null,
  selectedProject: string | null,
  selectedGroupId: string,
): ContainerSelection {
  if (preview)
    return {
      kind: "session",
      id: favKey(preview),
      title: preview.title,
      dir: preview.project_dir,
    }
  // 项目选中以 != null 判定("":未知项目桶,真值语义);分组选中按哨兵
  // 识别,其余 id 即具体组(含组刚被删的陈旧 id —— 由 label/切片端兜底)。
  if (selectedProject != null) return { kind: "project", id: selectedProject }
  if (selectedGroupId === UNGROUPED) return { kind: "ungrouped" }
  if (selectedGroupId === ALL_GROUPS) return { kind: "all" }
  return { kind: "group", id: selectedGroupId }
}

/** 容器的右栏口径 tag(#108 定稿三态):会话态 / 分组态(含未分组)/ 项目态
 *  ——未选任何容器时照旧共用项目卡组(无身份卡的全量聚合),不加第四档。 */
export function containerScopeTag(
  c: ContainerSelection,
): "session" | "project" | "group" {
  switch (c.kind) {
    case "session":
      return "session"
    case "group":
    case "ungrouped":
      return "group"
    case "project":
    case "all":
      return "project"
  }
}

/**
 * The container's display name — scopeLabel and the list pane's header share
 * this one rule (四层嵌套三元 ×2 的归属). 只产原始文本或 i18n 键,翻译由调用
 * 方 t() 完成(spanLabelKey 同一分工)。组名缺失(组刚被删的竞态)回落到
 * sessions.tree.all,与收编前各手写副本的 fallback 一致。
 */
export type ContainerLabel =
  | { text: string }
  | {
      key:
        | "sessions.untitled"
        | "sessions.tree.noProject"
        | "sessions.group.ungrouped"
        | "sessions.tree.all"
    }

export function containerLabel(
  c: ContainerSelection,
  groupNameOf: (id: string) => string | undefined,
): ContainerLabel {
  switch (c.kind) {
    case "session":
      return c.title ? { text: c.title } : { key: "sessions.untitled" }
    case "project": {
      const base = c.id ? projectBasename(c.id) : ""
      return base ? { text: base } : { key: "sessions.tree.noProject" }
    }
    case "group": {
      const name = groupNameOf(c.id)
      return name ? { text: name } : { key: "sessions.tree.all" }
    }
    case "ungrouped":
      return { key: "sessions.group.ungrouped" }
    case "all":
      return { key: "sessions.tree.all" }
  }
}

/**
 * 窄容器下拉的值编码(p:/g: DSL 的唯一归属):p:<identity> / g:<id> /
 * g:__ungrouped__ / ""(全部)。会话维度不进树——树上拉是树容器的镜像控件,
 * 调用方对它传 preview=null 的 resolveContainer 结果;parseTreeSelectValue 是
 * 它的逆映射,两者 round-trip 有测试钉住。
 */
export function treeSelectValue(c: ContainerSelection): string {
  switch (c.kind) {
    case "project":
      return `p:${c.id}`
    case "ungrouped":
      return `g:${UNGROUPED}`
    case "group":
      return `g:${c.id}`
    case "session":
    case "all":
      return ""
  }
}

export type TreeSelectAction =
  | { type: "all" }
  | { type: "project"; id: string }
  | { type: "group"; id: string }

export function parseTreeSelectValue(v: string): TreeSelectAction {
  if (!v) return { type: "all" }
  if (v.startsWith("p:")) return { type: "project", id: v.slice(2) }
  return { type: "group", id: v.slice(2) }
}

/**
 * 容器的统计行切片(一次聚合,三种容器同一口径——hook 里那条手写 if 链的
 * 归属)。会话态取整份宇宙读数:统计源本就是 selection-free 的全宇宙读取,
 * 会话卡的聚合只在其统计行缺席时兜底展示,退回全宇宙比受树选中抽样的伪切片
 * 更稳(见 stats-rail SessionCards 的 fallback 注释)。
 */
export function containerStatsRows(
  c: ContainerSelection,
  universe: readonly SessionStatsRow[],
  buckets: Readonly<{
    grouped: ReadonlyMap<string, readonly SessionStatsRow[]>
    ungrouped: readonly SessionStatsRow[]
  }>,
): readonly SessionStatsRow[] {
  switch (c.kind) {
    case "session":
    case "all":
      return universe
    case "project":
      return universe.filter((r) => r.project_dir === c.id)
    case "ungrouped":
      return buckets.ungrouped
    case "group":
      return buckets.grouped.get(c.id) ?? []
  }
}

// ------------------------------------------------------- workbench stats ----

/**
 * The four token buckets the workbench aggregates over — the JS mirror of the
 * Rust `TokenCounts` u32 four-pack; 形状与总量口径归 lib/token-buckets（架构
 * 审查候选⑨），此别名保留本域的接口词。
 */
export type StatsTokens = TokenBuckets

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
    const span = spanMsOf(r)
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
    node.tokens += totalTokensOf(r)
    if (r.last_active_at > node.lastActiveAt)
      node.lastActiveAt = r.last_active_at
    byProject.set(r.project_dir, node)
  }
  return [...byProject.values()].sort((a, b) =>
    b.lastActiveAt.localeCompare(a.lastActiveAt),
  )
}

/**
 * The group tracks' row bucketing: rows keyed by their group id, with every
 * row whose id is empty OR not in `knownIds` falling into `ungrouped` — a
 * stale id (group since deleted) is treated as ungrouped, never silently
 * dropped. The one rule, in the one place the count list reads.
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

// ------------------------------------------------- subagent nesting (#90) --

/**
 * Subagent nesting for the workbench's session list: rows carrying an explicit
 * `parent_session_id` whose parent row (same device, that id) is IN THE SAME
 * LIST move directly under it, so the list reads as structure rather than a
 * flat time-desc pile. Purely presentational — every row stays its own row
 * (details are never merged, tokens never move), the page count and the
 * backend ordering are untouched.
 *
 * Degradation is explicit and graceful: a child whose parent is absent from
 * the current page (filtered out, or an older page) keeps its top-level
 * position — it still shows the ↳ subagent marker, it just doesn't indent.
 * Nesting applies within the loaded slice only (the paged list's contract);
 * matching keys on the composite (device_id, id), never the bare id.
 */
export interface NestedSessions {
  /** Display order — children moved directly after their parent row. */
  rows: SessionRow[]
  /** favKeys of the rows rendered as indented children. */
  nestedKeys: Set<string>
}

export function nestSubagents(rows: readonly SessionRow[]): NestedSessions {
  // Index the in-slice children by their parent's composite key, then rebuild:
  // unclaimed rows keep their fetch order (time-desc), each immediately
  // followed by its children (recursively — real data is single-level, but a
  // deeper chain still renders rather than dropping rows).
  const present = new Set(rows.map(favKey))
  const childOf = new Map<string, SessionRow[]>()
  for (const r of rows) {
    if (!r.parent_session_id) continue
    const key = `${r.device_id}/${r.parent_session_id}`
    if (!present.has(key)) continue
    const bucket = childOf.get(key)
    if (bucket) bucket.push(r)
    else childOf.set(key, [r])
  }
  if (childOf.size === 0) {
    return { rows: [...rows], nestedKeys: new Set() }
  }
  const out: SessionRow[] = []
  const nestedKeys = new Set<string>()
  const claimed = new Set([...childOf.values()].flat().map((r) => favKey(r)))
  const emit = (r: SessionRow): void => {
    out.push(r)
    for (const child of childOf.get(favKey(r)) ?? []) {
      nestedKeys.add(favKey(child))
      emit(child)
    }
  }
  for (const r of rows) {
    if (claimed.has(favKey(r))) continue
    emit(r)
  }
  return { rows: out, nestedKeys }
}

// projectBasename moved to lib/paths (the project filter dropdown — a usage-
// feature surface — shares it); re-exported so the sessions call sites keep
// their import seam (same pattern as reorderIds below). containerLabel above
// also reads it for the project bucket's display name.
export { projectBasename }
