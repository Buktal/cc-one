// Sessions browser 组合根（架构审查Ⅴ拆分后）：只做三件事——公共状态与读数
// （track / search / 共享筛选 / 会话列表与统计查询 / 分页控制器 / 收藏乐观覆
// 盖）、三个子域 hook 的装配（分组域 useSessionGroups / 批量域
// useSessionBatch / 详情+邻步域 useSessionDetail）、以及跨域依赖的显式参数
// 注入。视图组件仍从本出口取全部键（形状与拆分前兼容，见 return 清单）。
//
// 跨域依赖只有三处，全部以显式参数表达（不再有 hook 间互读）：
// - selectProject / selectGroup / selectAll 复位详情（detail.setPreview(null)）；
// - 批量归组按轨道取 mutation（groups.groupMutations(effectiveTrack) 经
//   runSetGroup 参数注入 batch 域）；
// - 邻步的页缘翻页借用分页控制器（detail 域经 shiftPages 参数拿到
//   browser.shiftPages）。
//
// vitest runs in a node-only environment (no DOM — see vitest.config.ts), so
// renderHook is out of scope; the companion test guards that this module
// imports cleanly in node (it pulls the tauri-specta API + RTK Query hooks).

import { useEffect, useMemo, useState } from "react"
import {
  useAppInfoQuery,
  useListSessionsQuery,
  useSessionCountsQuery,
  useSessionStatsQuery,
  useSetSessionFavoritedMutation,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"
import { useDebouncedValue } from "@/hooks/use-debounced-value"
import { usePagedBrowser } from "@/hooks/use-paged-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { useDeviceLabels, useDeviceOptions } from "@/lib/device-labels"
import { usePersistedState } from "@/lib/persistence"
import { useProjectCandidates } from "@/lib/project-candidates"
import type { SessionRow } from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  aggregateStats,
  containerStatsRows,
  effectiveFavorite,
  favKey,
  type GroupTrack,
  groupedRows,
  identityOfProjectFilter,
  nestSubagents,
  nextFavValue,
  projectFilterOfIdentity,
  projectNodes,
  resolveContainer,
  type SessionScopeSpec,
  type TreeTrack,
  trackUniverseTab,
  withFavOverride,
  withoutFavOverride,
} from "./derive"
import { useSessionJumpConsumer } from "./session-jump"
import { useSessionBatch } from "./use-session-batch"
import { useSessionDetail } from "./use-session-detail"
import { useSessionGroups } from "./use-session-groups"

/** Persisted-track key — the tree track (项目 / 分组 / 收藏) survives
 *  restarts. Replaces the old Local/Favorites tab key: the track IS the
 *  universe switch now (定稿 §1). */
const TRACK_KEY = "cc-one:sessions-track"

/** Persisted page-size key — the center list's per-page density (三栏定稿：
 *  每页 20/50/100) survives restarts. Owned by usePagedBrowser (persistKey),
 *  这里只剩键名声明。 */
const PAGE_SIZE_KEY = "cc-one:sessions-page-size"

export function useSessionsBrowser() {
  const dispatch = useAppDispatch()
  // 树轨道（项目/分组/收藏）是页面的宇宙开关：前两轨读本机会话，收藏轨读
  // 跨设备收藏——tab（local/favorites）由轨道派生，仍是后端 scope 的语言。
  const [track, setTreeTrack] = usePersistedState<TreeTrack>(
    TRACK_KEY,
    "projects",
  )
  const tab = trackUniverseTab(track)
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

  // 分页控制器（架构扫描候选⑧ + Ⅴ）：offset / 页统计 / 翻页 / 密度持久化
  // 单一归属。scope 身份变化 → 回第 1 页——结构性规则，scope 里新增维度自
  // 动参与；密度（persistKey 托管）在控制器内折进身份，换页大小即维度变化，
  // 同一规则负责回第 1 页（offset 在不同页大小下不同义，不能沿用）。
  const browser = usePagedBrowser({
    scope,
    persistKey: PAGE_SIZE_KEY,
    total: viewCounts.total,
  })

  // ---- 分组域（子 hook）：双轨分组 CRUD + 分派表随迁；跨域参数 = 轨道 /
  // git 开关 / 选中组复位。 ----
  const groups = useSessionGroups({
    effectiveTrack,
    synced,
    selectedGroupId,
    onSelectedGroupDeleted: () => setSelectedGroupId(ALL_GROUPS),
  })

  // Paged session list (mirrors the request-log table). Skipped until
  // selfDeviceId resolves so the local tab never queries with an empty
  // device_scope.
  const sessionsQuery = useListSessionsQuery(
    { ...scope, limit: browser.pageSize, offset: browser.offset },
    { skip: !selfDeviceId },
  )

  // Drop optimistic overrides the moment fresh list data lands — the write's
  // invalidation triggered a refetch, so the real favorited value is now in.
  const sessionsData = sessionsQuery.data
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — clear overrides when fresh query data arrives; the body needs no sessionsData value
  useEffect(() => {
    setFavOverrides({})
  }, [sessionsData])

  const [favoritedMut] = useSetSessionFavoritedMutation()
  const runWithToast = useMutateWithToast()

  // ---- derived read model (pure functions from ./derive) ----
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
  const knownGroupIds = useMemo(
    () => new Set(groups.trackGroups.map((g) => g.id)),
    [groups.trackGroups],
  )
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
    const m = new Map<string, (typeof statsRows)[number]>()
    for (const r of statsRows) m.set(favKey(r), r)
    return m
  }, [statsRows])

  // ---- 详情+邻步域（子 hook）：详情目标复合键 / transcript 读数 / 邻步；
  // 跨域参数 = 当前页列表 + 分页控制器的 offset/pageSize/total/shiftPages。 ----
  const detail = useSessionDetail({
    visibleSessions,
    offset: browser.offset,
    pageSize: browser.pageSize,
    total: viewCounts.total,
    shiftPages: browser.shiftPages,
  })

  // ---- 容器选中（架构审查候选⑤）：「当前在看谁」的唯一编码在 ./derive 的
  // resolveContainer——会话 > 项目 > 分组 > 未分组 > 全部的阶梯由其分支次序
  // 表达、测试钉住；此处构造一份供本 hook 与视图层共用（b.container），统计
  // 切片（containerStatsRows）随之收敛：三种容器一次聚合。会话态切片 = 整份
  // 宇宙读数，见 containerStatsRows 注释。
  const container = useMemo(
    () => resolveContainer(detail.preview, selectedProject, selectedGroupId),
    [detail.preview, selectedProject, selectedGroupId],
  )
  const selectionStatsRows = useMemo(
    () => containerStatsRows(container, statsRows, groupBuckets),
    [container, statsRows, groupBuckets],
  )
  const selectionAggregate = useMemo(
    () => aggregateStats(selectionStatsRows),
    [selectionStatsRows],
  )

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
    detail.setPreview(null)
  }
  function selectGroup(groupId: string): void {
    setSelectedGroupId(groupId)
    detail.setPreview(null)
  }
  function selectAll(): void {
    setSelectedGroupId(ALL_GROUPS)
    // 「全部」清的是容器选中：分组 + 项目维度（项目即筛选，与其它维度同态
    // ——其它维度的「全部」也是清除）。
    dispatch(patchFilter({ project: "" }))
    detail.setPreview(null)
  }

  // 跨域跳转落地（usage 请求日志→会话，features/sessions/session-jump.ts）：
  // target 到达时取回会话行并经 detail.setPreview 打开——与列表行点击同一
  // 条通道。
  useSessionJumpConsumer(detail.setPreview)

  // ---- 设备标签面（架构审查候选⑥）：标签与选项表来自共享 lib/device-labels，
  // 本域不再手抄同一套 label 派生（is_self 有无的漂移即出自旧手抄版）。选项
  // 表自带「≤1 台返回 []」策略——设备列随之只在多设备的收藏轨出现。
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

  // ---- 批量域（子 hook）：勾选集 + 批量收藏/归组/删除。跨域参数 = 归组动
  // 作（组合根按 effectiveTrack 查分组域的分派表后注入——batch 域不持有任
  // 何轨道知识）。 ----
  const batch = useSessionBatch({
    runSetGroup: (target, groupId) =>
      groups
        .groupMutations(effectiveTrack)
        .setGroup({ id: target.id, deviceId: target.device_id, groupId }),
  })

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
    trackGroups: groups.trackGroups,
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
    pageSize: browser.pageSize,
    setPageSize: browser.density.onChange,
    // PaginationBar 密度选择器直连（usePagedBrowser 直出，视图不再拼 8 键）。
    density: browser.density,
    // device labels (favorites universe)
    deviceLabels,
    showDeviceColumn,
    // session row actions
    effectiveFavorite: (s: SessionRow) => effectiveFavorite(s, favOverrides),
    toggleFavorite,
    setSessionGroup: groups.setSessionGroup,
    // batch operations
    checkedCount: batch.checkedCount,
    isChecked: batch.isChecked,
    toggleCheck: batch.toggleCheck,
    clearChecked: batch.clearChecked,
    batchFavorite: batch.batchFavorite,
    batchSetGroup: batch.batchSetGroup,
    batchDelete: batch.batchDelete,
    // detail (选中会话)
    preview: detail.preview,
    setPreview: detail.setPreview,
    openNeighbor: detail.openNeighbor,
    canPrev: detail.canPrev,
    canNext: detail.canNext,
    transcript: detail.transcript,
    transcriptLoading: detail.transcriptLoading,
    transcriptError: detail.transcriptError,
    refetchTranscript: detail.refetchTranscript,
    // group CRUD
    createGroupOpen: groups.createGroupOpen,
    setCreateGroupOpen: groups.setCreateGroupOpen,
    openCreateGroup: groups.openCreateGroup,
    createGroup: groups.createGroup,
    renameGroup: groups.renameGroup,
    deleteGroup: groups.deleteGroup,
    reorderGroups: groups.reorderGroups,
    pendingGroup: groups.pendingGroup,
    busyGroupId: groups.busyGroupId,
  }
}

export type UseSessionsBrowser = ReturnType<typeof useSessionsBrowser>
