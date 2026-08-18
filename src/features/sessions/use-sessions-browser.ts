// Sessions browser state + actions, extracted from SessionsView so the
// component shrinks to pure rendering. Owns: the tab / search / sidebar-group
// selection, the session list + groups + devices queries, the transcript query
// for the open detail sheet, optimistic favorite toggling, the two-track group
// CRUD (local = immediate, synced = async git push → optimistic + loading), and
// the derived sidebar buckets / visible-session list.
//
// vitest runs in a node-only environment (no DOM — see vitest.config.ts), so
// renderHook is out of scope; the companion test guards that this module
// imports cleanly in node (it pulls the tauri-specta API + RTK Query hooks).

import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useAppInfoQuery,
  useCreateLocalGroupMutation,
  useCreateSyncedGroupMutation,
  useDeleteLocalGroupMutation,
  useDeleteSyncedGroupMutation,
  useDevicesQuery,
  useDistinctModelsQuery,
  useListGroupsQuery,
  useListSessionsQuery,
  useRenameLocalGroupMutation,
  useRenameSyncedGroupMutation,
  useReorderLocalGroupsMutation,
  useReorderSyncedGroupsMutation,
  useSessionCountsQuery,
  useSessionTranscriptQuery,
  useSetSessionCustomTitleMutation,
  useSetSessionFavoritedMutation,
  useSetSessionLocalGroupMutation,
  useSetSessionSyncedGroupMutation,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { type FilterState, patchFilter } from "@/app/store/slices/filterSlice"
import { setView } from "@/app/store/slices/viewSlice"
import { deviceOptionLabel } from "@/features/usage/use-device-options"
import { useDebouncedValue } from "@/hooks/use-debounced-value"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { facetOptions } from "@/lib/filter-options"
import { paginate } from "@/lib/pagination"
import { usePersistedState } from "@/lib/persistence"
import type { SessionGroup, SessionRow } from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  applyGroupOrder,
  canCreateSyncedGroup,
  effectiveFavorite,
  favKey,
  type GroupTrack,
  neighborNav,
  nextFavValue,
  type SessionScopeSpec,
  type SessionTab,
  ungroupedCount,
  withFavOverride,
  withoutFavOverride,
} from "./derive"

/** Persisted-tab key — the chosen tab (local / favorites) survives restarts. */
const TAB_KEY = "cc-one:sessions-tab"

/** Rows per page — matches the request-log table so both data views page at
 *  the same density. Exported for the view's paginator (the disabled state
 *  must agree with the query's page size — one source of truth). */
export const SESSIONS_PAGE_SIZE = 20

/** Title-rename 状态的单一归属（架构扫描候选⑨c）：detail sheet 的头部就地
 *  管理「编辑中 / 草稿 / 提交」，不再经 useSessionsBrowser → SessionDetailSheet
 *  → SessionHeader 逐层传递六个 props。rename mutation 与 toast 策略在此
 *  自己拿（RTK hooks 全局缓存 + useMutateWithToast 每次挂载独立，无共享态）。 */
export function useSessionTitleRename(session: SessionRow | null) {
  const [editTitle, setEditTitle] = useState(false)
  const [titleDraft, setTitleDraft] = useState("")
  const [customTitleMut] = useSetSessionCustomTitleMutation()
  const runWithToast = useMutateWithToast()

  function startEditTitle(): void {
    if (!session) return
    setEditTitle(true)
    setTitleDraft(session.title)
  }
  function cancelEditTitle(): void {
    setEditTitle(false)
  }
  async function commitEditTitle(): Promise<void> {
    if (!session) return
    const name = titleDraft.trim()
    // Empty draft = revert to the original title (clears the custom override).
    if (!name || name === session.title) {
      setEditTitle(false)
      return
    }
    const ok = await runWithToast(
      customTitleMut,
      { id: session.id, deviceId: session.device_id, title: name },
      {
        success: { key: "sessions.toast.renamed" },
        failed: { key: "sessions.toast.failed" },
      },
    )
    if (ok) setEditTitle(false)
  }
  return {
    editTitle,
    titleDraft,
    setTitleDraft,
    startEditTitle,
    cancelEditTitle,
    commitEditTitle,
  }
}

export function useSessionsBrowser() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const [tab, setTab] = usePersistedState<SessionTab>(TAB_KEY, "local")
  const [search, setSearch] = useState("")
  // Search is backend-side (the page query filters the whole set, not just
  // the loaded page), so keystrokes debounce before they hit the db.
  const debouncedSearch = useDebouncedValue(search, 300)
  // Common dimensions (time / model / source / device) live in the shared
  // filterSlice — sessions shares them with the dashboard / logs.
  // Only the sessions-only dimensions (tab / search / group) stay local here.
  const filter = useAppSelector((s) => s.filter.filter)
  const source = filter.source
  const model = filter.model
  const rangePreset = filter.range_preset
  const fromDay = filter.from_day
  const toDay = filter.to_day
  const deviceScope = filter.device_scope
  // Setters patch the shared slice so the view's contract (b.setSource / …) is
  // unchanged — the values now flow through Redux instead of local state.
  const setSource = (v: string) => dispatch(patchFilter({ source: v }))
  const setModel = (v: string) => dispatch(patchFilter({ model: v }))
  const setDeviceScope = (v: string) =>
    dispatch(patchFilter({ device_scope: v }))
  // Page offset into the filtered set (absolute row offset, like the request
  // log). Reset to page 1 whenever any filter dimension changes — otherwise a
  // narrower filter (search / range / source / model / device / group switch)
  // can land on an empty page.
  const [offset, setOffset] = useState(0)
  const [selectedGroupId, setSelectedGroupId] = useState<string>(ALL_GROUPS)
  const [favOverrides, setFavOverrides] = useState<Record<string, boolean>>({})
  const [pendingGroup, setPendingGroup] = useState<string | null>(null)
  const [busyGroupId, setBusyGroupId] = useState<string | null>(null)
  // Detail-sheet target stored as a composite key (device_id, id), not a row
  // snapshot. A snapshot goes stale the moment a favorite toggle's refetch
  // clears the optimistic override map — effectiveFavorite would then fall
  // back to the snapshot's old `favorited`, making the sheet's star flicker
  // back to its pre-toggle state. The derived `preview` (below) resolves this
  // key against the live sessions array every render, so it always carries the
  // freshest row.
  const [previewKey, setPreviewKey] = useState<{
    id: string
    device_id: string
  } | null>(null)
  // Last row seen for the open preview — fallback when the session leaves the
  // current slice (tab switch / filter change) so the sheet stays open instead
  // of snapping shut. Refreshed whenever the live lookup hits.
  const lastKnownRef = useRef<SessionRow | null>(null)
  const [createGroupOpen, setCreateGroupOpen] = useState(false)

  // The sidebar selection is track-scoped (local vs synced group ids are
  // disjoint spaces), so a tab switch must drop a stale selection.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset the group selection on tab switch; the body needs no tab value
  useEffect(() => {
    setSelectedGroupId(ALL_GROUPS)
  }, [tab])

  // Reset the page when any filter dimension changes. The common dimensions
  // live in the shared filterSlice now, so `filter` (a stable Redux reference
  // that only changes identity when a dimension actually changes) covers them;
  // the sessions-only dimensions (tab / search / group) are listed alongside.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset page on filter change; the body needs no dimension values
  useEffect(() => {
    setOffset(0)
  }, [filter, tab, debouncedSearch, selectedGroupId])

  const { data: appInfo } = useAppInfoQuery()
  const selfDeviceId = appInfo?.device_id ?? ""
  const synced = appInfo?.mode === "synced"
  const effectiveTrack: GroupTrack = tab === "local" ? "local" : "synced"

  // Time-range setters patch the shared slice. A dynamic preset stores no
  // concrete date; manual date edits flip to "custom" with literal
  // days.
  function setRangePreset(p: FilterState["range_preset"]): void {
    dispatch(patchFilter({ range_preset: p, from_day: "", to_day: "" }))
  }
  function patchFromDay(d: string): void {
    dispatch(patchFilter({ range_preset: "custom", from_day: d }))
  }
  function patchToDay(d: string): void {
    dispatch(patchFilter({ range_preset: "custom", to_day: d }))
  }
  // No timestamp derivation or cross-midnight timer here: the session endpoints
  // take a SessionScopeSpec (no timestamp) and derive the bounds in their
  // queryFn at query time. Midnight rollover rides the collect-
  // interval refresh chain, same as the usage views.

  // One scope for both reads: the common dimensions (from the shared
  // filterSlice) + the sessions-only dimensions (tab / search / group). The
  // backend SessionFilter + timestamp bounds are derived from it in the
  // endpoint queryFn (buildSessionFilter), so this object carries no
  // timestamp and its cache key (sessionSpecId) stays stable across a day.
  // selfDeviceId is part of the scope (not a filter dimension) because the
  // Local tab narrows to it backend-side.
  const scope: SessionScopeSpec = {
    filter,
    tab,
    selfDeviceId,
    selectedGroupId,
    search: debouncedSearch || null,
  }
  // Model-dropdown candidates mirror the usage view's facet semantics: the
  // sessions model list comes from usage_records (a session has no model column
  // — the list query filters by EXISTS), so narrow by the time / source / device
  // window but never by model itself. A FilterState facet (model cleared) feeds
  // the endpoint, which derives bounds at query time.
  const modelFacetFilter = useMemo(() => ({ ...filter, model: "" }), [filter])
  const { data: distinctModels = [] } = useDistinctModelsQuery(modelFacetFilter)
  // 并回规则（已选模型并入候选，窗口切换后下拉不空）收敛在 facetOptions。
  const modelOptions = useMemo(
    () => facetOptions(distinctModels, model),
    [distinctModels, model],
  )
  // Paged session list (mirrors the request-log table). Skipped until
  // selfDeviceId resolves so the local tab never queries with an empty
  // device_scope.
  const sessionsQuery = useListSessionsQuery(
    { ...scope, limit: SESSIONS_PAGE_SIZE, offset },
    { skip: !selfDeviceId },
  )
  // 两套计数，语义不同：
  // - 侧栏计数（分组分布 + 「全部」行）：**全局聚合，不含选中分组**——选中
  //   分组只是过滤右侧列表，侧栏分布不该跟着变（否则切到 A 组时 B/C/D 的
  //   数字全部归零，明显错乱）。
  // - 视图计数（分页 total）：跟随当前分组——列表已被分组过滤，分页总数
  //   必须匹配列表范围，否则翻页错位。
  const sidebarCountsQuery = useSessionCountsQuery(
    { spec: { ...scope, selectedGroupId: ALL_GROUPS }, track: effectiveTrack },
    { skip: !selfDeviceId },
  )
  const viewCountsQuery = useSessionCountsQuery(
    { spec: scope, track: effectiveTrack },
    { skip: !selfDeviceId },
  )
  const sidebarCounts = sidebarCountsQuery.data ?? { total: 0, groups: [] }
  const viewCounts = viewCountsQuery.data ?? { total: 0, groups: [] }
  const groupsQuery = useListGroupsQuery()
  const groups = groupsQuery.data ?? []
  const { data: devices = [] } = useDevicesQuery()
  const transcriptQuery = useSessionTranscriptQuery(
    previewKey
      ? { id: previewKey.id, deviceId: previewKey.device_id }
      : { id: "", deviceId: "" },
    { skip: !previewKey },
  )

  // Drop optimistic overrides the moment fresh list data lands — the write's
  // invalidation triggered a refetch, so the real favorited value is now in.
  const sessionsData = sessionsQuery.data
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — clear overrides when fresh query data arrives; the body needs no sessionsData value
  useEffect(() => {
    setFavOverrides({})
  }, [sessionsData])

  // Same pattern for the group-drag override: cleared when the reorder's
  // invalidation delivers the real order.
  const [groupOrderOverride, setGroupOrderOverride] = useState<string[] | null>(
    null,
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — clear overrides when fresh query data arrives; the body needs no groupsData value
  useEffect(() => {
    setGroupOrderOverride(null)
  }, [groupsQuery.data])

  const [favoritedMut] = useSetSessionFavoritedMutation()
  const [setLocalGroupMut] = useSetSessionLocalGroupMutation()
  const [setSyncedGroupMut] = useSetSessionSyncedGroupMutation()
  const [createLocalMut] = useCreateLocalGroupMutation()
  const [renameLocalMut] = useRenameLocalGroupMutation()
  const [deleteLocalMut] = useDeleteLocalGroupMutation()
  const [createSyncedMut] = useCreateSyncedGroupMutation()
  const [renameSyncedMut] = useRenameSyncedGroupMutation()
  const [deleteSyncedMut] = useDeleteSyncedGroupMutation()
  const [reorderLocalMut] = useReorderLocalGroupsMutation()
  const [reorderSyncedMut] = useReorderSyncedGroupsMutation()
  const runWithToast = useMutateWithToast()

  // ---- derived read model (pure functions from ./derive) ----
  // Natural order comes sorted from the backend; the override re-sorts it
  // optimistically while a drag's write is in flight.
  const trackGroups = useMemo(
    () =>
      applyGroupOrder(
        groups.filter((g) => g.kind === effectiveTrack),
        groupOrderOverride,
      ),
    [groups, effectiveTrack, groupOrderOverride],
  )
  // Sidebar counts from the backend aggregation: the per-bucket map (group
  // rows) and the derived ungrouped count (total minus known buckets — stale
  // ids count as ungrouped, the rule the old client-side grouping applied).
  // 用全局聚合（sidebarCounts）——切分组不改变侧栏分布。
  const groupCounts = useMemo(() => {
    const m = new Map<string, number>()
    for (const g of sidebarCounts.groups) m.set(g.group_id, g.count)
    return m
  }, [sidebarCounts])
  const knownGroupIds = useMemo(
    () => new Set(trackGroups.map((g) => g.id)),
    [trackGroups],
  )
  const ungroupedN = useMemo(
    () => ungroupedCount(sidebarCounts, knownGroupIds),
    [sidebarCounts, knownGroupIds],
  )
  // The visible list is the backend's current page — already narrowed by the
  // tab/toolbar/search AND the sidebar group selection, time-desc ordered.
  const visibleSessions = sessionsQuery.data ?? []

  // Page stats for the footer control (clamped so a shrunken result set can't
  // leave the paginator pointing past the end). 分页用视图计数（viewCounts，
  // 跟随分组过滤）；侧栏「全部」行用全局计数（sidebarCounts.total）。
  const { totalPages, page } = paginate(
    viewCounts.total,
    offset,
    SESSIONS_PAGE_SIZE,
  )

  // sessions lookup by composite key — O(1) resolve for the derived preview.
  // Reuses the favKey shape ("device_id/id") so favorite + preview agree on
  // identity (a session is uniquely (device_id, id)). Only the current page
  // is in memory; a preview whose row left the slice falls back to the
  // last-known row below so the sheet stays open across page turns.
  const sessionsByKey = useMemo(() => {
    const m = new Map<string, SessionRow>()
    for (const s of visibleSessions) m.set(favKey(s), s)
    return m
  }, [visibleSessions])

  // Derived preview: resolve the open key against the live sessions array
  // every render. After a favorite toggle's refetch this picks up the fresh
  // row immediately, so effectiveFavorite(preview) reflects the new value
  // instead of flickering back to a stale snapshot. Falls back to the
  // last-known row when the session has left the current slice (tab switch /
  // filter) so the detail sheet stays open.
  const livePreview = useMemo<SessionRow | null>(() => {
    if (!previewKey) return null
    return sessionsByKey.get(favKey(previewKey)) ?? null
  }, [previewKey, sessionsByKey])
  // Refresh the fallback only on a live hit; when the row leaves the slice the
  // fallback keeps the previous row so the sheet does not snap shut.
  useEffect(() => {
    if (livePreview) lastKnownRef.current = livePreview
  }, [livePreview])
  const preview = previewKey ? (livePreview ?? lastKnownRef.current) : null

  // setPreview keeps the caller contract (SessionRow | null) but stores only
  // the composite key — so the transcript query and title/favorite lookups
  // keep working even after a tab switch or filter change removes the row
  // from the visible list.
  function setPreview(s: SessionRow | null): void {
    if (s) {
      lastKnownRef.current = s
      setPreviewKey({ id: s.id, device_id: s.device_id })
    } else {
      lastKnownRef.current = null
      setPreviewKey(null)
    }
  }

  // ---- detail sheet: prev / next session navigation ----
  // Walks the currently visible page (±1 row). At a page edge the step pages
  // into the adjacent page and opens its target row (next → first row of the
  // next page, prev → last row of the previous page) once the new page's data
  // lands — see the pendingNeighbor effect below.
  const pendingNeighbor = useRef<{ delta: 1 | -1; fromKey: string } | null>(
    null,
  )
  const neighbor = useMemo(
    () =>
      neighborNav(
        visibleSessions,
        previewKey ? favKey(previewKey) : null,
        offset,
        SESSIONS_PAGE_SIZE,
        viewCounts.total,
      ),
    [visibleSessions, previewKey, offset, viewCounts.total],
  )

  function openNeighbor(delta: 1 | -1): void {
    if (!preview) return
    const idx = visibleSessions.findIndex((s) => favKey(s) === favKey(preview))
    if (idx === -1) return // preview left the visible page (filter changed)
    const target = visibleSessions[idx + delta]
    if (target) {
      setPreview(target)
      return
    }
    // Page edge with a page beyond it: flip the page, open the target row
    // when the new data arrives. Bounded by neighborNav's canPrev/canNext, so
    // the button is disabled when there is nowhere to go.
    pendingNeighbor.current = { delta, fromKey: favKey(preview) }
    setOffset(offset + delta * SESSIONS_PAGE_SIZE)
  }

  // Consume the pending page-edge step when the new page's data lands. Guarded
  // by fromKey: if the user switched to another row while the page loaded, the
  // pending step is dropped instead of hijacking their selection.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — setPreview is stable (reads refs only); adding it would re-run the effect every render
  useEffect(() => {
    const p = pendingNeighbor.current
    if (!p) return
    if (!previewKey || favKey(previewKey) !== p.fromKey) return
    const target =
      p.delta === 1
        ? visibleSessions[0]
        : visibleSessions[visibleSessions.length - 1]
    if (!target) return // new page still loading — wait for the next change
    pendingNeighbor.current = null
    setPreview(target)
  }, [visibleSessions, previewKey])

  // id → display label for the favorites tab's source-device column. Self is
  // "This device"; a peer is its display name (or "Unnamed").
  const deviceLabel = useMemo(() => {
    const m = new Map<string, string>()
    for (const d of devices) {
      m.set(d.device_id, deviceOptionLabel(d, t))
    }
    return m
  }, [devices, t])
  // Device-picker options for the Favorites-tab device filter. Same label logic
  // as deviceLabel; empty when ≤1 device so a single-machine setup renders no
  // device filter (mirrors the usage view's useDeviceOptions).
  const deviceOptions = useMemo(
    () =>
      devices.length <= 1
        ? []
        : devices.map((d) => ({
            id: d.device_id,
            label: deviceOptionLabel(d, t),
          })),
    [devices, t],
  )
  // The device column only matters in the favorites tab, and only when there
  // is more than one device (otherwise every row is "This device" — noise).
  const showDeviceColumn = tab === "favorites" && devices.length > 1

  // ---- session row actions ----
  // Favorite toggle is an optimistic state machine (stamp → mutate → rollback
  // on failure); the decisions live in ./derive (effectiveFavorite /
  // nextFavValue / withFavOverride / withoutFavOverride) so they are unit-
  // tested. This hook only wires them to React state + the mutation.
  async function toggleFavorite(s: SessionRow): Promise<void> {
    const next = nextFavValue(s, favOverrides)
    setFavOverrides((p) => withFavOverride(p, s, next))
    const ok = await runWithToast(
      favoritedMut,
      { id: s.id, deviceId: s.device_id, favorited: next },
      {
        success: {
          key: next ? "sessions.toast.favorited" : "sessions.toast.unfavorited",
        },
        failed: { key: "sessions.toast.failed" },
      },
    )
    if (!ok) {
      // Rollback the optimistic flip.
      setFavOverrides((p) => withoutFavOverride(p, s))
    }
  }

  /** group 双轨 mutation 选择（架构扫描候选⑨b）：local（SQLite 直写）vs
   *  synced（git 往返）——四个 group mutation 的成对关系收敛在此，调用方只
   *  选轨道：操作当前标签的组（setSessionGroup / reorderGroups）传
   *  effectiveTrack；操作指定组（renameGroup / deleteGroup）传该组的 kind。
   *  create 不在表里：其返回类型两轨不同（LocalGroup vs SyncedGroup），
   *  union trigger 无法喂给 runWithToast 的泛型推断，createGroup 保留手动
   *  分支。新增 group mutation 时成对关系必须进这张表。 */
  function groupMutations(track: GroupTrack) {
    return track === "local"
      ? {
          setGroup: setLocalGroupMut,
          rename: renameLocalMut,
          delete: deleteLocalMut,
          reorder: reorderLocalMut,
        }
      : {
          setGroup: setSyncedGroupMut,
          rename: renameSyncedMut,
          delete: deleteSyncedMut,
          reorder: reorderSyncedMut,
        }
  }

  async function setSessionGroup(
    s: SessionRow,
    groupId: string | null,
  ): Promise<void> {
    const mut = groupMutations(effectiveTrack).setGroup
    await runWithToast(
      mut,
      { id: s.id, deviceId: s.device_id, groupId },
      {
        success: { key: "sessions.toast.groupAssigned" },
        failed: { key: "sessions.toast.failed" },
      },
    )
  }

  // ---- group CRUD ----
  // Local groups are immediate (SQLite); synced groups round-trip through git
  // push, so the UI shows an optimistic pending row + spinner until the write
  // resolves (ADR 0002).
  //
  // Synced (Favorites-tab) groups need a bound Git repo — without one the
  // create would silently fail or hang. openCreateGroup is the UX guard (toast
  // + a one-hop to Settings, never opens the dialog); createGroup re-checks
  // defensively in case a caller bypasses the opener.
  function notifyGitRequired(): void {
    toast.warning(t("sessions.group.gitRequiredTitle"), {
      description: t("sessions.group.gitRequiredDesc"),
      action: {
        label: t("sessions.group.gitRequiredAction"),
        onClick: () => dispatch(setView("settings")),
      },
    })
  }
  function openCreateGroup(): void {
    if (!canCreateSyncedGroup(effectiveTrack, synced)) {
      notifyGitRequired()
      return
    }
    setCreateGroupOpen(true)
  }
  async function createGroup(name: string): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) return false
    if (!canCreateSyncedGroup(effectiveTrack, synced)) {
      notifyGitRequired()
      return false
    }
    // create 的返回类型两轨不同（LocalGroup vs SyncedGroup），union trigger
    // 无法喂给 runWithToast 的泛型推断（TS 逆变的 trigger 参数不接受联合），
    // 分支保留在此（groupMutations 表里无 create 条目，见其注释）。
    setPendingGroup(trimmed)
    const ok =
      effectiveTrack === "local"
        ? await runWithToast(createLocalMut, trimmed, {
            success: { key: "sessions.toast.groupCreated" },
            failed: { key: "sessions.toast.failed" },
          })
        : await runWithToast(createSyncedMut, trimmed, {
            success: { key: "sessions.toast.groupCreated" },
            failed: { key: "sessions.toast.failed" },
          })
    setPendingGroup(null)
    if (ok) setCreateGroupOpen(false)
    return ok
  }

  async function renameGroup(g: SessionGroup, name: string): Promise<void> {
    const trimmed = name.trim()
    if (!trimmed || trimmed === g.name) return
    setBusyGroupId(g.id)
    try {
      // 组自身的轨道决定走哪套 mutation（groups.json 里来的 synced 组与本地
      // 组共存于同一侧栏）。
      const mut = groupMutations(g.kind === "local" ? "local" : "synced").rename
      await runWithToast(
        mut,
        { id: g.id, name: trimmed },
        {
          success: { key: "sessions.toast.groupRenamed" },
          failed: { key: "sessions.toast.failed" },
        },
      )
    } finally {
      setBusyGroupId(null)
    }
  }

  async function deleteGroup(g: SessionGroup): Promise<void> {
    setBusyGroupId(g.id)
    try {
      const mut = groupMutations(g.kind === "local" ? "local" : "synced").delete
      const ok = await runWithToast(mut, g.id, {
        success: { key: "sessions.toast.groupDeleted" },
        failed: { key: "sessions.toast.failed" },
      })
      if (ok && selectedGroupId === g.id) setSelectedGroupId(ALL_GROUPS)
    } finally {
      setBusyGroupId(null)
    }
  }

  // Group drag-reorder: optimistic stamp → mutate → snap back on failure. A
  // drag must not visibly snap while the write is in flight (synced reorders
  // round-trip through git), and the outcome is already visible to the user —
  // no success toast.
  async function reorderGroups(orderedIds: string[]): Promise<void> {
    setGroupOrderOverride(orderedIds)
    const mut = groupMutations(effectiveTrack).reorder
    const ok = await runWithToast(mut, orderedIds, {
      failed: { key: "sessions.toast.failed" },
    })
    if (!ok) setGroupOrderOverride(null)
  }

  return {
    // tab / search / source / model / selection
    tab,
    setTab,
    search,
    setSearch,
    source,
    setSource,
    model,
    setModel,
    modelOptions,
    selectedGroupId,
    setSelectedGroupId,
    effectiveTrack,
    // toolbar filters (time range · device)
    rangePreset,
    fromDay,
    toDay,
    setRangePreset,
    setFromDay: patchFromDay,
    setToDay: patchToDay,
    deviceScope,
    setDeviceScope,
    deviceOptions,
    // data
    isLoading: sessionsQuery.isLoading,
    isFetching: sessionsQuery.isFetching,
    error: sessionsQuery.error,
    trackGroups,
    visibleSessions,
    // paging + sidebar counts
    // totalCount = 侧栏「全部」行（全局聚合，切分组不变）；viewTotal = 分页
    // 总数（跟随当前分组过滤的列表范围）。
    totalCount: sidebarCounts.total,
    viewTotal: viewCounts.total,
    page,
    totalPages,
    offset,
    setOffset,
    groupCounts,
    ungroupedCount: ungroupedN,
    // device labels (favorites tab)
    deviceLabel,
    showDeviceColumn,
    // session row actions
    effectiveFavorite: (s: SessionRow) => effectiveFavorite(s, favOverrides),
    toggleFavorite,
    setSessionGroup,
    // detail sheet
    preview,
    setPreview,
    openNeighbor,
    canPrev: neighbor.canPrev,
    canNext: neighbor.canNext,
    transcript: transcriptQuery.data ?? [],
    transcriptLoading: transcriptQuery.isLoading,
    transcriptError: transcriptQuery.error,
    refetchTranscript: transcriptQuery.refetch,
    // group CRUD
    createGroupOpen,
    setCreateGroupOpen,
    openCreateGroup,
    createGroup,
    renameGroup,
    deleteGroup,
    reorderGroups,
    pendingGroup,
    busyGroupId,
  }
}

export type UseSessionsBrowser = ReturnType<typeof useSessionsBrowser>
