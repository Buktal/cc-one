// Sessions browser state + actions, extracted from SessionsView so the
// component shrinks to pure rendering. Owns: the track / search / sidebar-group
// selection, the per-page size, the session list + groups + devices queries,
// the transcript query for the open detail sheet, optimistic favorite
// toggling, the two-track group CRUD (local = immediate, synced = async git
// push → optimistic + loading), and the derived sidebar buckets /
// visible-session list.
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
  useDeleteSessionsMutation,
  useDeleteSyncedGroupMutation,
  useListGroupsQuery,
  useListSessionsQuery,
  useRenameLocalGroupMutation,
  useRenameSyncedGroupMutation,
  useReorderLocalGroupsMutation,
  useReorderSyncedGroupsMutation,
  useSessionCountsQuery,
  useSessionStatsQuery,
  useSessionTranscriptQuery,
  useSetSessionFavoritedMutation,
  useSetSessionLocalGroupMutation,
  useSetSessionSyncedGroupMutation,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"
import { setView } from "@/app/store/slices/viewSlice"
import { useDebouncedValue } from "@/hooks/use-debounced-value"
import { usePagedBrowser } from "@/hooks/use-paged-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { useDeviceLabels, useDeviceOptions } from "@/lib/device-labels"
import { DEFAULT_PAGE_SIZE } from "@/lib/pagination"
import { usePersistedState } from "@/lib/persistence"
import { useProjectCandidates } from "@/lib/project-candidates"
import type {
  SessionGroup,
  SessionRow,
  SessionStatsRow,
} from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  aggregateStats,
  applyGroupOrder,
  canCreateSyncedGroup,
  containerStatsRows,
  effectiveFavorite,
  favKey,
  type GroupTrack,
  groupedRows,
  identityOfProjectFilter,
  neighborNav,
  nestSubagents,
  nextFavValue,
  type PendingNeighborStep,
  planNeighborStep,
  projectFilterOfIdentity,
  projectNodes,
  resolveContainer,
  type SessionScopeSpec,
  settleNeighborStep,
  type TreeTrack,
  trackUniverseTab,
  withFavOverride,
  withoutFavOverride,
} from "./derive"
import { useSessionJumpConsumer } from "./session-jump"

/** Persisted-track key — the tree track (项目 / 分组 / 收藏) survives
 *  restarts. Replaces the old Local/Favorites tab key: the track IS the
 *  universe switch now (定稿 §1). */
const TRACK_KEY = "cc-one:sessions-track"

/** Persisted page-size key — the center list's per-page density (三栏定稿：
 *  每页 20/50/100) survives restarts. */
const PAGE_SIZE_KEY = "cc-one:sessions-page-size"

export function useSessionsBrowser() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  // 树轨道（项目/分组/收藏）是页面的宇宙开关：前两轨读本机会话，收藏轨读
  // 跨设备收藏——tab（local/favorites）由轨道派生，仍是后端 scope 的语言。
  const [track, setTreeTrack] = usePersistedState<TreeTrack>(
    TRACK_KEY,
    "projects",
  )
  const tab = trackUniverseTab(track)
  const [pageSize, setPageSize] = usePersistedState<number>(
    PAGE_SIZE_KEY,
    DEFAULT_PAGE_SIZE,
  )
  const [search, setSearch] = useState("")
  // Search is backend-side (the page query filters the whole set, not just
  // the loaded page), so keystrokes debounce before they hit the db.
  const debouncedSearch = useDebouncedValue(search, 300)
  // Common dimensions (time / model / source / device / project) live in the
  // shared filterSlice — sessions shares them with the dashboard / logs. Only
  // the sessions-only dimensions (track / search / group selection) stay local
  // here.
  const filter = useAppSelector((s) => s.filter.filter)
  // 项目维度与左树项目轨道统一（#102）：树的项目桶选中和工具栏的项目下拉
  // 是同一份状态——共享 filterSlice.project。selectedProject（视图契约不变）
  // 由筛选值映射回树的 identity 空间（哨兵 → "" 无启动目录桶）；哨兵值从候选
  // 端点以数据过线，见 useProjectCandidates。
  const { unknownValue } = useProjectCandidates()
  const selectedProject = identityOfProjectFilter(filter.project, unknownValue)
  // 左树的两级选中：分组选中是轨道语义的（local/synced 组 id 不共空间），项目
  // 选中（= 筛选维度）跨轨存续——切轨只复位分组选中。
  const [selectedGroupId, setSelectedGroupId] = useState<string>(ALL_GROUPS)
  const [favOverrides, setFavOverrides] = useState<Record<string, boolean>>({})
  const [pendingGroup, setPendingGroup] = useState<string | null>(null)
  const [busyGroupId, setBusyGroupId] = useState<string | null>(null)
  // Detail target stored as a composite key (device_id, id), not a row
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
  // 批量操作：勾选键 = favKey（device/id 复合键），值保留行的定位信息——
  // 勾选可跨页留存，批量动作不依赖「行恰好在当前页」。
  const [checked, setChecked] = useState<
    Map<string, { id: string; device_id: string }>
  >(() => new Map())

  // The tree selection is track-scoped (local vs synced group ids are disjoint
  // spaces), so a track switch must drop a stale group selection. The project
  // dimension needs no reset — it lives in the shared filter and persists
  // across tracks like source / model / device do.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset the selection on track switch; the body needs no track value
  useEffect(() => {
    setSelectedGroupId(ALL_GROUPS)
  }, [track])

  const { data: appInfo } = useAppInfoQuery()
  const selfDeviceId = appInfo?.device_id ?? ""
  const synced = appInfo?.mode === "synced"
  const effectiveTrack: GroupTrack = tab === "local" ? "local" : "synced"

  // No timestamp derivation or cross-midnight timer here: the session endpoints
  // take a SessionScopeSpec (no timestamp) and derive the bounds in their
  // queryFn at query time. Midnight rollover rides the collect-
  // interval refresh chain, same as the usage views.

  // One scope for the reads: the common dimensions (from the shared
  // filterSlice, project included) + the sessions-only dimensions (track
  // universe / search / group selection). The backend SessionFilter +
  // timestamp bounds are derived from it in the endpoint queryFn
  // (buildSessionFilter), so this object carries no timestamp and its cache
  // key (sessionSpecId) stays stable across a day. selfDeviceId is part of the
  // scope (not a filter dimension) because the Local universe narrows to it
  // backend-side.
  const scope: SessionScopeSpec = {
    filter,
    tab,
    selfDeviceId,
    selectedGroupId,
    search: debouncedSearch || null,
  }
  // The stats reads are CONTAINER-SELECTION-FREE (All groups, project
  // dropped): the tree and the right rail need the whole universe at once —
  // they bucket client-side, and the project dimension is the projects tree's
  // own container dimension (selecting a bucket must not collapse the tree,
  // the same facet rule the candidate endpoint applies). Same toolbar
  // dimensions otherwise, so a search / time change refilters both the list
  // and the stats consistently.
  const universeScope: SessionScopeSpec = {
    ...scope,
    selectedGroupId: ALL_GROUPS,
    filter: { ...scope.filter, project: "" },
  }
  // 视图计数（分页 total）：跟随当前分组——列表已被分组过滤，分页总数必须
  // 匹配列表范围，否则翻页错位。（左栏计数清单是 selection-free 的 statsRows
  // 分桶，不走这份带选中的计数——两套口径见上。）
  const viewCountsQuery = useSessionCountsQuery(
    { spec: scope, track: effectiveTrack },
    { skip: !selfDeviceId },
  )
  const viewCounts = viewCountsQuery.data ?? { total: 0, groups: [] }
  // 工作台统计读（会话粒度）：左树计数、右栏全部卡组同一次 selection-free
  // 读——一条统计路径，无第二份口径。
  const statsQuery = useSessionStatsQuery(universeScope, {
    skip: !selfDeviceId,
  })
  const statsRows = statsQuery.data ?? []

  // 分页控制器（架构扫描候选⑧）：offset / 页统计 / 翻页单一归属。scope 身份
  // 变化 → 回第 1 页——结构性规则，scope 里新增维度自动参与，不再手列依赖
  // 清单（此前 4 组互不相同的手列数组各自编码同一不变量）。pageSize 挂在
  // browser 的 scope 里：每页条数切换即「维度变化」，同一规则负责回第 1 页
  // （offset 在不同页大小下不同义，不能沿用）。
  const browser = usePagedBrowser({
    scope: { ...scope, pageSize },
    pageSize,
    total: viewCounts.total,
  })
  // Paged session list (mirrors the request-log table). Skipped until
  // selfDeviceId resolves so the local tab never queries with an empty
  // device_scope.
  const sessionsQuery = useListSessionsQuery(
    { ...scope, limit: pageSize, offset: browser.offset },
    { skip: !selfDeviceId },
  )

  const groupsQuery = useListGroupsQuery()
  const groups = groupsQuery.data ?? []
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
  const [deleteSessionsMut] = useDeleteSessionsMutation()
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
  // knownGroupIds：分组桶的已知组集合——空串与陈旧 id（组已删）都归未分组。
  const knownGroupIds = useMemo(
    () => new Set(trackGroups.map((g) => g.id)),
    [trackGroups],
  )
  // The visible list is the backend's current page — already narrowed by the
  // track-universe/toolbar/search AND the tree's container selection (group or
  // project), time-desc ordered — then NESTED (#90): subagent rows carrying an
  // explicit parent link move directly under their in-page parent. Purely
  // presentational (rows stay separate; the page count is untouched), and the
  // display order IS the navigation order — the table, the detail's prev/next
  // walk, and the preview lookup all read this one list.
  const nested = useMemo(
    () => nestSubagents(sessionsQuery.data ?? []),
    [sessionsQuery.data],
  )
  const visibleSessions = nested.rows

  // ---- 左树计数清单数据（selection-free 统计行的客户端分桶）----
  // 项目轨：projectNodes 已按桶最近活跃排序；分组/收藏轨：knownIds 之外的
  // 组 id（含空串）落入未分组桶。
  const projectBuckets = useMemo(() => projectNodes(statsRows), [statsRows])
  const groupBuckets = useMemo(
    () =>
      groupedRows(
        statsRows,
        (r) =>
          effectiveTrack === "local" ? r.local_group_id : r.synced_group_id,
        knownGroupIds,
      ),
    [statsRows, effectiveTrack, knownGroupIds],
  )
  // stats row lookup by the same composite key the list uses — the right
  // rail's「按会话」卡 resolves the selected session against it.
  const statsByKey = useMemo(() => {
    const m = new Map<string, SessionStatsRow>()
    for (const r of statsRows) m.set(favKey(r), r)
    return m
  }, [statsRows])
  // 右栏「按项目」聚合对象：项目选中 = 该项目桶（identity "" = 无启动目录
  // 桶，哨兵映射后）；分组选中 = 该组行；未选中 = 全量。一次聚合，三种容器
  // 同一口径。（架构审查候选⑤：切片规则与视图侧口径对象共用 ./derive 的
  // containerStatsRows / resolveContainer，不再有第二份手写 if 链。）

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

  // ---- 容器选中（架构审查候选⑤）：「当前在看谁」的唯一编码在 ./derive 的
  // resolveContainer——会话 > 项目 > 分组 > 未分组 > 全部的阶梯由其分支次序
  // 表达、测试钉住；此处构造一份供本 hook 与视图层共用（b.container），统计
  // 切片（containerStatsRows）随之收敛：三种容器一次聚合。会话态切片 = 整份
  // 宇宙读数，见 containerStatsRows 注释。
  const container = useMemo(
    () => resolveContainer(preview, selectedProject, selectedGroupId),
    [preview, selectedProject, selectedGroupId],
  )
  const selectionStatsRows = useMemo(
    () => containerStatsRows(container, statsRows, groupBuckets),
    [container, statsRows, groupBuckets],
  )
  const selectionAggregate = useMemo(
    () => aggregateStats(selectionStatsRows),
    [selectionStatsRows],
  )

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

  // ---- 树选中动作：容器选中即让出会话态（右栏口径随之回到容器态——口径
  // 由「是否打开会话」派生，不再手设）。项目选中写共享筛选维度（与工具栏
  // 下拉同态）：identity 非空 → 原样；""（无启动目录桶）→ 哨兵值（端点数
  // 据；未知时退化为不约束，见 projectFilterOfIdentity）；null → 清除。
  function selectProject(project: string | null): void {
    dispatch(
      patchFilter({
        project:
          project == null ? "" : projectFilterOfIdentity(project, unknownValue),
      }),
    )
    if (project) setTreeTrack("projects")
    setPreviewKey(null)
  }
  function selectGroup(groupId: string): void {
    setSelectedGroupId(groupId)
    setPreviewKey(null)
  }
  function selectAll(): void {
    setSelectedGroupId(ALL_GROUPS)
    // 「全部」清的是容器选中：分组 + 项目维度（项目即筛选，与其它维度同态
    // ——其它维度的「全部」也是清除）。
    dispatch(patchFilter({ project: "" }))
    setPreviewKey(null)
  }

  // 跨域跳转落地（usage 请求日志→会话，features/sessions/session-jump.ts）：
  // target 到达时取回会话行并经 setPreview 打开——与列表行点击同一条通道。
  useSessionJumpConsumer(setPreview)

  // ---- detail sheet: prev / next session navigation ----
  // Walks the currently visible page (±1 row); at a page edge it pages into
  // the adjacent page and opens its target row once the new page's data
  // lands. The decisions live in ./derive (planNeighborStep /
  // settleNeighborStep — the stepping invariants are unit-tested there); this
  // hook only registers the pending step, shifts pages, and consumes the
  // settlement when the list data changes.
  const pendingNeighbor = useRef<PendingNeighborStep | null>(null)
  const neighbor = useMemo(
    () =>
      neighborNav(
        visibleSessions,
        previewKey ? favKey(previewKey) : null,
        browser.offset,
        pageSize,
        viewCounts.total,
      ),
    [visibleSessions, previewKey, browser.offset, pageSize, viewCounts.total],
  )

  function openNeighbor(delta: 1 | -1): void {
    const step = planNeighborStep(
      visibleSessions,
      previewKey ? favKey(previewKey) : null,
      delta,
      browser.offset,
      pageSize,
      viewCounts.total,
    )
    if (step.kind === "in-page") {
      setPreview(step.target)
    } else if (step.kind === "page-edge") {
      pendingNeighbor.current = step.pending
      browser.shiftPages(delta)
    }
  }

  // Consume the pending page-edge step when the new page's data lands (the
  // open / drop / wait rules live in settleNeighborStep).
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — setPreview is stable (reads refs only); adding it would re-run the effect every render
  useEffect(() => {
    const p = pendingNeighbor.current
    if (!p) return
    const settled = settleNeighborStep(
      p,
      previewKey ? favKey(previewKey) : null,
      visibleSessions,
    )
    if (settled.kind === "wait") return
    pendingNeighbor.current = null
    if (settled.kind === "open") setPreview(settled.target)
  }, [visibleSessions, previewKey])

  // 设备标签面（架构审查候选⑥）：标签与选项表来自共享 lib/device-labels，本域
  // 不再手抄同一套 label 派生（is_self 有无的漂移即出自旧手抄版）。选项表自带
  // 「≤1 台返回 []」策略——设备列随之只在多设备的收藏轨出现。
  const deviceLabels = useDeviceLabels()
  const deviceOptions = useDeviceOptions()
  const showDeviceColumn = tab === "favorites" && deviceOptions.length > 0

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

  // ---- 批量操作（定稿 §6：勾选后批量收藏 / 归组 / 删除）----
  // 勾选键 = favKey，值保留 (id, device_id) 定位——批量动作不依赖行还在
  // 当前页（勾选可跨页留存）。收藏/归组对全部勾选行并发执行，结束一条
  // 汇总 toast（逐行 toast 会在大勾选下刷屏）；删除走单条批量命令。
  function toggleCheck(s: SessionRow): void {
    setChecked((prev) => {
      const next = new Map(prev)
      const key = favKey(s)
      if (next.has(key)) next.delete(key)
      else next.set(key, { id: s.id, device_id: s.device_id })
      return next
    })
  }
  function clearChecked(): void {
    setChecked(new Map())
  }
  function isChecked(s: SessionRow): boolean {
    return checked.has(favKey(s))
  }
  async function runBatch(
    run: (target: { id: string; device_id: string }) => Promise<unknown>,
    successKey: string,
  ): Promise<void> {
    const targets = [...checked.values()]
    const results = await Promise.allSettled(targets.map((t) => run(t)))
    const failed = results.filter((r) => r.status === "rejected").length
    if (failed === 0) {
      toast.success(t(successKey, { n: targets.length }))
    } else {
      toast.warning(t("sessions.toast.batchPartial", { n: failed }))
    }
    clearChecked()
  }
  async function batchFavorite(): Promise<void> {
    await runBatch(
      (t) => favoritedMut({ id: t.id, deviceId: t.device_id, favorited: true }),
      "sessions.toast.batchFavorited",
    )
  }
  async function batchSetGroup(groupId: string | null): Promise<void> {
    const mut = groupMutations(effectiveTrack).setGroup
    await runBatch(
      (t) => mut({ id: t.id, deviceId: t.device_id, groupId }),
      "sessions.toast.batchGrouped",
    )
  }
  // 批量删除（#91）：一次命令带全部勾选键（后端批量软删除——排除标记随
  // 采集/拉取稳定，源文件不动）；确认对话在工具条（BatchBar）里，动作
  // 只在被确认后到达这里。返回值（实际命中行数）驱动成功 toast。
  async function batchDelete(): Promise<void> {
    const targets = [...checked.values()]
    if (targets.length === 0) return
    await runWithToast(deleteSessionsMut, targets, {
      success: {
        message: (n) => t("sessions.toast.batchDeleted", { n }),
      },
      failed: { key: "sessions.toast.failed" },
    })
    clearChecked()
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
    // track (tree rail) / search / selection。source / model 维度（含候选下拉）
    // 由视图经共享 useFacetDimension 直连全局 filter，不再走本出口。
    track,
    setTreeTrack,
    search,
    setSearch,
    selectedGroupId,
    selectGroup,
    selectedProject,
    selectProject,
    // 容器选中（架构审查候选⑤）：会话/项目/分组/未分组/全部的判别联合，
    // 视图层的口径 tag、口径名、列表头、窄容器下拉都从它派生。
    container,
    selectAll,
    effectiveTrack,
    // 工具条的五维筛选（时间/来源/模型/项目/设备）全部读写共享 filterSlice，
    // 由视图层的共享 FilterBar 直接接线（架构审查Ⅳ候选⑫）——时间经
    // useDateRangeFilter、设备显隐门控（收藏轨）在 FilterBar 的 showDevice，
    // 均不再经本出口；时间窗的后端语义随 spec.filter 生效。
    // data
    isLoading: sessionsQuery.isLoading,
    isFetching: sessionsQuery.isFetching,
    error: sessionsQuery.error,
    trackGroups,
    visibleSessions,
    // #90 缩进展示：被挂到父行下的子行 favKey 集合（表行渲染缩进的依据）。
    nestedSessionKeys: nested.nestedKeys,
    // 左树（两级）+ 右栏统计
    statsRows,
    projectBuckets,
    groupBuckets,
    statsByKey,
    selectionRows: selectionStatsRows,
    selectionAggregate,
    // paging
    // viewTotal = 分页总数（跟随当前分组过滤的列表范围）；左栏计数清单的
    // 「全部」行读 statsRows.length（selection-free，同一份统计源）。
    viewTotal: viewCounts.total,
    page: browser.page,
    totalPages: browser.totalPages,
    goToPage: browser.goToPage,
    pageSize,
    setPageSize,
    // device labels (favorites universe)
    deviceLabels,
    showDeviceColumn,
    // session row actions
    effectiveFavorite: (s: SessionRow) => effectiveFavorite(s, favOverrides),
    toggleFavorite,
    setSessionGroup,
    // batch operations
    checkedCount: checked.size,
    isChecked,
    toggleCheck,
    clearChecked,
    batchFavorite,
    batchSetGroup,
    batchDelete,
    // detail (选中会话)
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
