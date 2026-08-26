// Sessions view —— 三栏工作台容器（#108 定稿 variant-a「图标轨道 + 计数
// 清单」，原型 docs/prototype/sessions-v2/variant-a-icon-rail-counts.html）：
// 左栏（图标轨道条 + 纯计数清单）｜中内容区（分页会话列表 / 会话详情）｜
// 右统计卡栏（口径随选中对象派生：会话态四卡 / 项目态项目卡 / 分组态轻量
// 汇总）。顶部工具条：时间 pill（作用于全部统计）+ 筛选 + 搜索 + 批量操作
// 入口。
//
// 中内容区两态：列表态（未选会话）= 分页会话列表（每页 20/50/100）；会话态
// = 标题行 + 对话流（session-detail，其余统计全在右栏）。窄容器让位分两档：
// < 48rem（768 档）左树先收成工具条下拉；< 58rem（928 档）右栏再折叠为
// 浮动按钮开抽屉（门槛推导见 stats-rail）——导航优先级高于统计。
//
// Pure rendering only — all state, queries, mutations and the optimistic
// favorite / pending-group handling live in useSessionsBrowser (./use-sessions-
// browser). This component owns JSX, styles, i18n and the source-display helper
// (../source-labels). Mirrors library-view.tsx's split.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { MessagesSquare, Search, Star, Trash2, X } from "lucide-react"
import { type ReactNode, useState } from "react"
import { useTranslation } from "react-i18next"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { DateRangeChip } from "@/components/date-range-chip"
import { FilterSelect } from "@/components/filter-select"
import { PaginationBar } from "@/components/pagination-bar"
import { QueryState } from "@/components/query-state"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { ProjectSelect } from "@/features/usage/components/project-select"
import type { FilterOption } from "@/lib/filter-options"
import {
  formatCost,
  formatCount,
  formatMetricSeg,
  formatTokens,
} from "@/lib/format"
import { PAGE_SIZES } from "@/lib/pagination"
import { SOURCE_TAGS } from "@/lib/source-tags"
import { cn } from "@/lib/utils"
import type { SessionRow } from "@/types/generated/bindings"
import {
  containerLabel,
  containerScopeTag,
  favKey,
  parseTreeSelectValue,
  projectBasename,
  resolveContainer,
  treeSelectValue,
} from "../derive"
import { highlight } from "../highlight"
import { sessionAgentKind, sessionSourceLabel } from "../source-labels"
import { useSessionsBrowser } from "../use-sessions-browser"
import { GroupCreateDialog } from "./group-create-dialog"
import { SessionDetail } from "./session-detail"
import { SessionTree } from "./session-tree"
import { NarrowStatsTrigger, StatsRail, type StatsScopeTag } from "./stats-rail"

dayjs.extend(relativeTime)

export function SessionsView() {
  const b = useSessionsBrowser()
  const { t } = useTranslation()
  // Narrow `preview` to a non-null local so the detail callbacks capture
  // a SessionRow, not SessionRow | null (TS will not narrow across callbacks
  // that read the field later).
  const preview = b.preview

  // 容器选中（架构审查候选⑤）：b.container 是「当前在看谁」的唯一编码，右栏
  // 口径 tag、口径名与中栏列表头是同一判别联合的两个读端；优先级阶梯（会话 >
  // 项目 > 分组 > 未分组 > 全部）住在 derive.resolveContainer 的分支次序里。
  const containerLbl = containerLabel(
    b.container,
    (id) => b.trackGroups.find((g) => g.id === id)?.name,
  )
  const scopeLabel =
    "text" in containerLbl ? containerLbl.text : t(containerLbl.key)
  const sessionStats = preview
    ? (b.statsByKey.get(favKey(preview)) ?? null)
    : null
  // 右栏口径 tag 与项目态身份卡数据：口径由选中对象派生，与 selectionAggregate
  // 的容器切片（containerStatsRows）共用同一份联合——不再有第二份手设。
  const scopeTag: StatsScopeTag = containerScopeTag(b.container)
  const projectIdentity =
    b.container.kind === "project"
      ? {
          dir: b.container.id,
          subagents: b.selectionRows.filter((r) => r.agent_type !== "").length,
        }
      : null
  // 列表态（无会话打开）的头部：标题读同一份容器联合，副题仅项目桶显示路径。
  const headDesc = b.container.kind === "project" ? b.container.id : ""

  // 统计数据切片：右栏本体与窄容器浮卡入口（hover 出小卡）消费同一份。
  const statsData = {
    scopeTag,
    scopeLabel,
    aggregate: b.selectionAggregate,
    session: preview,
    sessionStats,
    transcript: b.transcript,
    transcriptLoading: b.transcriptLoading,
    deviceLabel: (id: string) => b.deviceLabel.get(id) ?? id.slice(0, 8),
    projectIdentity,
  }

  return (
    // @container/sessions 是工作台唯一的响应式坐标系：60rem 导航收窄条 /
    // 48rem 左树上台 / 76rem 右栏上台（四列几何真正并存且留余量的宽度，见
    // stats-rail 头注；右栏恒宽 256px 不再分级加宽）。所有组件（含中列内部
    // 的轮次导航与统计图标）
    // 一律用 @min-*/@max-*/sessions: 引用这把尺——曾各找最近祖先容器，两层
    // 嵌套尺让「图标与右栏同屏」「右栏在而导航没了」先后翻车。左树 ≥48rem
    // 恒在、不让位（曾让树在 ≥58 让位——那覆盖了几乎所有真实窗口，等于树
    // 常年消失，已撤）。主导航折叠不牵动它。
    // 高度模型：本视图在外壳是 fill 型（App 的 FILL_VIEWS 直挂 main flex
    // 列），高度严格 = 视口剩余空间、无外层滚动；下面整条链 min-h-0 +
    // 各面板自带滚动容器。
    <div className="@container/sessions flex min-h-0 flex-1 flex-col gap-3">
      <WorkbenchToolbar b={b} />

      <div className="flex min-h-0 flex-1 gap-3">
        <SessionTree
          track={b.track}
          onTrackChange={b.setTreeTrack}
          statsRows={b.statsRows}
          projectBuckets={b.projectBuckets}
          groupBuckets={b.groupBuckets}
          trackGroups={b.trackGroups}
          selectedGroupId={b.selectedGroupId}
          selectedProject={b.selectedProject}
          onSelectAll={b.selectAll}
          onSelectProject={b.selectProject}
          onSelectGroup={b.selectGroup}
          onCreateGroup={b.openCreateGroup}
          onRenameGroup={b.renameGroup}
          onDeleteGroup={b.deleteGroup}
          onReorderGroups={b.reorderGroups}
          pendingGroup={b.pendingGroup}
          busyGroupId={b.busyGroupId}
        />

        {/* 中内容区（两态：列表 / 会话详情）。不再自带 @container——历史上
            这把「第二把尺」让详情内部的组件量中列宽度、与外层档位错位；
            显隐与压缩一律引用外层的 /sessions 命名容器。 */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col">
          {preview ? (
            <SessionDetail
              session={preview}
              favorited={b.effectiveFavorite(preview)}
              onClose={() => b.setPreview(null)}
              onToggleFavorite={() => b.toggleFavorite(preview)}
              trackGroups={b.trackGroups}
              currentGroupId={
                b.effectiveTrack === "local"
                  ? preview.local_group_id
                  : preview.synced_group_id
              }
              onSetGroup={(groupId) => b.setSessionGroup(preview, groupId)}
              transcript={b.transcript}
              transcriptLoading={b.transcriptLoading}
              transcriptError={b.transcriptError}
              onRefreshTranscript={b.refetchTranscript}
              onPrev={() => b.openNeighbor(-1)}
              onNext={() => b.openNeighbor(1)}
              canPrev={b.canPrev}
              canNext={b.canNext}
              statsSlot={<NarrowStatsTrigger {...statsData} />}
            />
          ) : (
            <ListPane
              b={b}
              headTitle={scopeLabel}
              headDesc={headDesc}
              statsSlot={<NarrowStatsTrigger {...statsData} />}
            />
          )}
        </div>

        <StatsRail {...statsData} />
      </div>

      <GroupCreateDialog
        open={b.createGroupOpen}
        onClose={() => b.setCreateGroupOpen(false)}
        onCreate={b.createGroup}
        creating={b.pendingGroup !== null}
        track={b.effectiveTrack}
      />
    </div>
  )
}

// ------------------------------------------------------------- 工具条 ----

/** 顶部工具条：筛选下拉全部居左（与看板 ControlBar 同序：时间 · 来源 · 模型
 *  · 项目 · 设备），搜索 + 批量操作居右；窄容器在时间后追加树容器下拉。
 *  统计入口不在此条——它是内容口径随选中对象走，放在卡片头/详情标题行的
 *  NarrowStatsTrigger 上。单行不换行——chips 内容自适应（FilterSelect 统一
 *  策略），一行放不下由工具条横向滚动兜底（scrollbar-none 隐轨道）。 */
function WorkbenchToolbar({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
  return (
    <div className="scrollbar-none flex items-center gap-2 overflow-x-auto">
      <div className="flex shrink-0 items-center gap-2">
        <DateRangeChip
          preset={b.rangePreset}
          fromDay={b.fromDay}
          toDay={b.toDay}
          onPreset={b.setRangePreset}
          onFromDay={b.setFromDay}
          onToDay={b.setToDay}
        />
        {/* 窄档（左树 <48rem 未上台）的容器下拉。 */}
        <div className="hidden @max-[48rem]/sessions:block">
          <NarrowTreeSelect b={b} />
        </div>
        <FilterSelect
          ariaLabel={t("sessions.filter.source")}
          allLabel={t("sessions.filter.allSources")}
          options={SOURCE_OPTIONS}
          value={b.source}
          onChange={b.setSource}
        />
        <FilterSelect
          ariaLabel={t("sessions.filter.model")}
          allLabel={t("sessions.filter.allModels")}
          options={b.modelOptions.map((m) => ({ value: m, label: m }))}
          value={b.model}
          onChange={b.setModel}
        />
        {/* 项目维度：共享 filterSlice（与看板 / 日志一致），左树项目轨道的
            选中也写同一份状态。候选取自 distinct-projects 端点，含「未知
            项目」特殊选项。 */}
        <ProjectSelect />
        {/* Device dropdown — only in the favorites universe (收藏轨）and only
            when more than one device exists. */}
        {b.deviceOptions.length > 0 && b.track === "favorites" ? (
          <FilterSelect
            ariaLabel={t("sessions.filter.device")}
            allLabel={t("sessions.filter.allDevices")}
            options={b.deviceOptions.map((o) => ({
              value: o.id,
              label: o.label,
            }))}
            value={b.deviceScope}
            onChange={b.setDeviceScope}
          />
        ) : null}
      </div>

      {/* 右：批量操作 + 搜索。宽裕时右贴（ml-auto 在无剩余空间时自然失效，
          不阻止滚动）。 */}
      <div className="ml-auto flex shrink-0 items-center gap-2">
        <BatchBar b={b} />
        <div className="relative w-40 @min-[48rem]/sessions:w-56">
          <Search className="text-muted-foreground absolute top-1/2 left-2 size-3.5 -translate-y-1/2" />
          <Input
            value={b.search}
            onChange={(e) => b.setSearch(e.target.value)}
            placeholder={t("sessions.searchPlaceholder")}
            className="h-8 pl-7"
            aria-label={t("sessions.searchPlaceholder")}
          />
        </div>
      </div>
    </div>
  )
}

/** 批量操作入口（定稿 §6）：勾选后出现——批量收藏 / 归组（下拉选组）/
 *  删除（#91 软删除，ConfirmDialog 二次确认后执行）+ 清除多选。 */
function BatchBar({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
  const [deleteOpen, setDeleteOpen] = useState(false)
  if (b.checkedCount === 0) return null
  return (
    <div className="flex items-center gap-1.5">
      <span className="bg-accent-tint text-accent-brand-strong rounded-full px-2 py-0.5 text-xs tabular-nums">
        {t("sessions.batch.selected", { n: b.checkedCount })}
      </span>
      <Button
        variant="outline"
        size="sm"
        onClick={() => void b.batchFavorite()}
      >
        <Star className="size-3.5" />
        {t("sessions.batch.favorite")}
      </Button>
      <FilterSelect
        ariaLabel={t("sessions.batch.group")}
        allLabel={t("sessions.batch.group")}
        options={b.trackGroups.map((g) => ({ value: g.id, label: g.name }))}
        value=""
        onChange={(v) => void b.batchSetGroup(v || null)}
        triggerSize="sm"
      />
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="outline"
              size="sm"
              onClick={() => setDeleteOpen(true)}
            >
              <Trash2 className="size-3.5" />
              {t("sessions.batch.delete")}
            </Button>
          }
        />
        <TooltipContent>{t("sessions.batch.deleteHint")}</TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("sessions.batch.clear")}
              onClick={b.clearChecked}
              className="text-muted-foreground"
            />
          }
        >
          <X className="size-3.5" />
        </TooltipTrigger>
        <TooltipContent>{t("sessions.batch.clear")}</TooltipContent>
      </Tooltip>
      <ConfirmDialog
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title={t("sessions.delete.title")}
        description={t("sessions.delete.desc", { n: b.checkedCount })}
        confirmLabel={t("sessions.delete.confirm")}
        onConfirm={() => {
          // Close-then-delete：确认即关对话再执行，结果由汇总 toast 反馈。
          setDeleteOpen(false)
          void b.batchDelete()
        }}
      />
    </div>
  )
}

/** 窄容器的容器下拉：全部 + 当前轨道的容器（项目 basename / 组名）。树上拉
 *  是树容器的镜像控件，会话维度不进树——resolveContainer 传 null 得到纯树视
 *  图；p:/g: 值编码与反解（treeSelectValue / parseTreeSelectValue）收口在
 *  derive。项目 identity 为 ""（未知项目桶）时编码为 "p:"，round-trip 安全。 */
function NarrowTreeSelect({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
  const options: FilterOption[] =
    b.track === "projects"
      ? b.projectBuckets.map((n) => ({
          value: `p:${n.project}`,
          label: projectBasename(n.project) || t("sessions.tree.noProject"),
        }))
      : b.trackGroups.map((g) => ({ value: `g:${g.id}`, label: g.name }))
  const value = treeSelectValue(
    resolveContainer(null, b.selectedProject, b.selectedGroupId),
  )
  return (
    <FilterSelect
      ariaLabel={t("sessions.tree.all")}
      allLabel={t("sessions.tree.all")}
      options={options}
      value={value}
      onChange={(v) => {
        const action = parseTreeSelectValue(v)
        if (action.type === "all") b.selectAll()
        else if (action.type === "project") b.selectProject(action.id)
        else b.selectGroup(action.id)
      }}
      fallbackLabel={t("sessions.tree.all")}
    />
  )
}

// ------------------------------------------------------------- 中栏 ----

/** 列表态骨架：头部（标题 + 项目路径 + 会话数 + 窄容器统计入口）+ 表格 +
 *  分页（每页 20/50/100）。标题/副题由视图层从容器联合派生后下发（同一份
 *  containerLabel，无第二份判定链）。项目统计头已上移右栏（项目态项目卡），
 *  中栏只管列表。 */
function ListPane({
  b,
  headTitle,
  headDesc,
  statsSlot,
}: {
  b: ReturnType<typeof useSessionsBrowser>
  headTitle: string
  headDesc: string
  statsSlot: ReactNode
}) {
  const { t } = useTranslation()

  return (
    // gap-0 py-0 推掉 Card 基类的 py-(--card-spacing)=20px 节距（与详情卡
    // 同手法）——内距全权由 CardContent 的 p-3 负责，两态头部节奏一致。
    <Card className="flex min-h-0 min-w-0 flex-1 flex-col gap-0 py-0">
      <CardContent className="flex min-h-0 flex-1 flex-col gap-2 p-3">
        <div className="flex shrink-0 items-center gap-2.5 px-0.5">
          <h3 className="text-sm font-semibold">{headTitle}</h3>
          {headDesc ? (
            <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs">
              {headDesc}
            </span>
          ) : null}
          {/* DSL 段：会话数 N（与分页条同源的 viewTotal）；其后是窄容器统计
              入口（宽容器整体隐身，故正常态此行右端就是会话数）。 */}
          <span className="text-muted-foreground ml-auto shrink-0 text-xs tabular-nums">
            {formatMetricSeg(
              t("sessions.stats.sessions"),
              formatCount(b.viewTotal),
            )}
          </span>
          {statsSlot}
        </div>

        <QueryState
          isLoading={b.isLoading}
          error={b.error}
          isEmpty={!b.isLoading && b.visibleSessions.length === 0}
          emptyIcon={MessagesSquare}
          // Empty means different things per universe: 项目/分组轨 = nothing
          // collected yet (go run a CLI), 收藏轨 = nothing starred yet. Same
          // copy for both would mislead.
          emptyLabel={
            b.search
              ? t("sessions.noMatch")
              : b.track === "favorites"
                ? t("sessions.empty.favoritesTitle")
                : t("sessions.empty.title")
          }
          emptyDescription={
            b.search
              ? undefined
              : b.track === "favorites"
                ? t("sessions.empty.favoritesDesc")
                : t("sessions.empty.desc")
          }
        >
          <SessionsTable
            rows={b.visibleSessions}
            effectiveFavorite={b.effectiveFavorite}
            onToggleFavorite={b.toggleFavorite}
            onOpen={b.setPreview}
            showDeviceColumn={b.showDeviceColumn}
            deviceLabel={b.deviceLabel}
            openFavKey={b.preview ? favKey(b.preview) : null}
            search={b.search}
            isChecked={b.isChecked}
            onToggleCheck={b.toggleCheck}
            showProjectColumn={b.selectedProject == null}
            nestedKeys={b.nestedSessionKeys}
          />
        </QueryState>

        <PaginationBar
          page={b.page}
          totalPages={b.totalPages}
          total={b.viewTotal}
          loading={b.isFetching}
          onPageChange={b.goToPage}
          pageSize={{
            value: b.pageSize,
            options: PAGE_SIZES,
            onChange: b.setPageSize,
          }}
        />
      </CardContent>
    </Card>
  )
}

function SessionsTable({
  rows,
  effectiveFavorite,
  onToggleFavorite,
  onOpen,
  showDeviceColumn,
  deviceLabel,
  openFavKey,
  search,
  isChecked,
  onToggleCheck,
  showProjectColumn,
  nestedKeys,
}: {
  rows: SessionRow[]
  effectiveFavorite: (s: SessionRow) => boolean
  onToggleFavorite: (s: SessionRow) => void
  onOpen: (s: SessionRow) => void
  showDeviceColumn: boolean
  deviceLabel: Map<string, string>
  /** favKey of the open detail row — that row gets a tinted selected state. */
  openFavKey: string | null
  /** Live search box value — matched title spans get highlighted. */
  search: string
  isChecked: (s: SessionRow) => boolean
  onToggleCheck: (s: SessionRow) => void
  /** 项目态下项目列冗余（表头已示项目与路径），隐藏让位给标题列。 */
  showProjectColumn: boolean
  /** #90 缩进展示：挂到父行下的子行 favKey 集合（nestSubagents 的输出）。 */
  nestedKeys: Set<string>
}) {
  const { t } = useTranslation()
  return (
    // 唯一滚动容器：Table 自带的 table-container wrapper（overflow-x-auto）
    // 会被 contents 推平，横向滚动条因此落在本容器底部——行数少时也贴着
    // 视口底，而不是悬在表内容底下。pr-2 给 8px 竖向滚动条让位，-mr-2 抵
    // 消常态下的右侧空隙。
    <div className="min-h-0 flex-1 -mr-2 overflow-auto pr-2 [&>[data-slot=table-container]]:contents">
      {/* table-fixed: column widths come from the header row, so the narrow
          numeric columns are never stretched by extra horizontal space. min-w
          keeps the title readable below the fixed sum — the outer overflow-auto
          scrolls horizontally instead of squeezing columns into overlap. */}
      {/* 列宽按内容收紧（类型/设备/项目各缩一档），min-w 相应 58→53rem：
          更小的窗口也能整表放下，标题列的剩余空间少挤一点。 */}
      <Table className="table-fixed min-w-[53rem]">
        <TableHeader>
          <TableRow>
            {/* 图标槽列（默认星标；悬停时让位给批量勾选框，见 TableCell）。 */}
            <TableHead className="w-9" />
            <TableHead className="max-w-[24rem]">
              {t("sessions.col.title")}
            </TableHead>
            <TableHead className="w-24">{t("sessions.col.type")}</TableHead>
            {showDeviceColumn ? (
              <TableHead className="w-24">{t("sessions.col.device")}</TableHead>
            ) : null}
            {showProjectColumn ? (
              <TableHead className="w-40">
                {t("sessions.col.project")}
              </TableHead>
            ) : null}
            <TableHead className="w-24">
              {t("sessions.col.lastActive")}
            </TableHead>
            <TableHead className="w-20 text-right">
              {t("sessions.col.requests")}
            </TableHead>
            <TableHead className="w-24 text-right">
              {t("sessions.col.tokens")}
            </TableHead>
            <TableHead className="w-20 text-right">
              {t("sessions.col.cost")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((s) => {
            const fav = effectiveFavorite(s)
            const open = openFavKey === favKey(s)
            const sub = s.agent_type !== ""
            // #90 缩进：显式父子链接且父行在同页的子行缩进挂到父行下
            //（nestSubagents 已把它移到父行正后方），缩进量落在 ↳ 标记上；
            // 父行不在同页的子行保持顶格，仅以 ↳ 标记类型。表格本身 min-w
            // + 横向滚动，三档容器宽度下缩进都不破版。
            const nested = nestedKeys.has(favKey(s))
            return (
              <TableRow
                key={favKey(s)}
                // group/row 驱动图标槽的悬停换位；selected 行 hover 也保持品
                // 牌底色（默认 hover:bg-hover 会闪灰盖过它）。
                className={cn(
                  "group/row",
                  open && "bg-accent-tint hover:bg-accent-tint",
                )}
              >
                <TableCell>
                  {/* 单一图标槽：静止显星标；悬停时批量勾选框原位淡入替换星
                      标，已勾选恒显勾选框——不为复选框保留整列空占位（用户
                      定稿）。透明态 pointer-events 关闭，避免挡住下层星标。
                      DOM 常驻，测试与键盘路径不受影响。 */}
                  <span className="relative flex size-6 items-center justify-center">
                    <Tooltip>
                      <TooltipTrigger
                        render={
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            aria-label={
                              fav
                                ? t("sessions.row.unfavorite")
                                : t("sessions.row.favorite")
                            }
                            onClick={(e: React.MouseEvent) => {
                              e.stopPropagation()
                              onToggleFavorite(s)
                            }}
                            className={cn("absolute", isChecked(s) && "hidden")}
                          />
                        }
                      >
                        <Star
                          className={cn(
                            "size-4",
                            fav
                              ? "fill-accent-brand text-accent-brand"
                              : "text-muted-foreground",
                          )}
                        />
                      </TooltipTrigger>
                      <TooltipContent>
                        {fav
                          ? t("sessions.row.unfavorite")
                          : t("sessions.row.favorite")}
                      </TooltipContent>
                    </Tooltip>
                    <Checkbox
                      checked={isChecked(s)}
                      onCheckedChange={() => onToggleCheck(s)}
                      aria-label={t("sessions.batch.check", {
                        title: s.title || t("sessions.untitled"),
                      })}
                      className={cn(
                        "absolute transition-opacity",
                        !isChecked(s) &&
                          "pointer-events-none opacity-0 group-hover/row:pointer-events-auto group-hover/row:opacity-100 focus-visible:pointer-events-auto focus-visible:opacity-100",
                      )}
                    />
                  </span>
                </TableCell>
                <TableCell>
                  {/* trackCursorAxis: the trigger is the full column width, so
                      a centered tooltip would float over the column's middle
                      (wrong for short titles) — anchor it to the cursor. */}
                  <Tooltip trackCursorAxis="both">
                    <TooltipTrigger
                      render={
                        <button
                          type="button"
                          className="hover:text-accent-brand-strong flex w-full min-w-0 flex-col items-start gap-0.5 text-left"
                          onClick={() => onOpen(s)}
                        />
                      }
                    >
                      <span className="block w-full min-w-0 truncate font-medium">
                        {sub ? (
                          <span
                            className={cn(
                              "text-muted-foreground/50 mr-0.5 select-none",
                              nested && "ml-3.5",
                            )}
                          >
                            ↳
                          </span>
                        ) : null}
                        {highlight(s.title || t("sessions.untitled"), search)}
                      </span>
                      <span className="text-muted-foreground text-xs">
                        {sessionSourceLabel(s.source)}
                      </span>
                    </TooltipTrigger>
                    <TooltipContent className="max-w-md">
                      {highlight(s.title || t("sessions.untitled"), search)}
                    </TooltipContent>
                  </Tooltip>
                </TableCell>
                <TableCell>
                  {(() => {
                    const kind = sessionAgentKind(s.agent_type)
                    // Every non-main row is a subagent, so the chip shows the
                    // bare agent type (e.g. Explore) — no "subagent" prefix.
                    // Color mirrors the request-log stop-reason palette:
                    // main = 常规主会话绿 (success), subagent = 派生工具活动
                    // 琥珀 (tool). Long types truncate within the column.
                    const label =
                      kind.kind === "main" ? t("sessions.type.main") : kind.type
                    return (
                      <Tooltip>
                        <TooltipTrigger
                          render={
                            <span
                              className={cn(
                                "sem-chip max-w-full",
                                kind.kind === "main" ? "type-main" : "type-sub",
                              )}
                            >
                              <span className="min-w-0 truncate">{label}</span>
                            </span>
                          }
                        />
                        <TooltipContent>{label}</TooltipContent>
                      </Tooltip>
                    )
                  })()}
                </TableCell>
                {showDeviceColumn ? (
                  <TableCell>
                    <span className="text-muted-foreground text-xs">
                      {deviceLabel.get(s.device_id) ?? s.device_id.slice(0, 8)}
                    </span>
                  </TableCell>
                ) : null}
                {showProjectColumn ? (
                  <TableCell className="text-muted-foreground text-xs">
                    <Tooltip trackCursorAxis="both">
                      <TooltipTrigger
                        render={<span className="block min-w-0 truncate" />}
                      >
                        {projectBasename(s.project_dir) || "—"}
                      </TooltipTrigger>
                      <TooltipContent className="max-w-sm break-all">
                        {s.project_dir || "—"}
                      </TooltipContent>
                    </Tooltip>
                  </TableCell>
                ) : null}
                <TableCell className="text-muted-foreground text-xs">
                  {s.last_active_at ? (
                    <Tooltip>
                      <TooltipTrigger
                        render={
                          <span>{dayjs(s.last_active_at).fromNow()}</span>
                        }
                      />
                      <TooltipContent>
                        {dayjs(s.last_active_at).format("YYYY-MM-DD HH:mm")}
                      </TooltipContent>
                    </Tooltip>
                  ) : (
                    "—"
                  )}
                </TableCell>
                <TableCell className="text-right text-xs tabular-nums">
                  {formatCount(s.request_count)}
                </TableCell>
                {/* Tokens + cost are the two numbers a usage tool is scanned
                    for — half-bold them so they read above the request count. */}
                <TableCell className="text-right text-xs font-medium tabular-nums">
                  {formatTokens(s.total_tokens)}
                </TableCell>
                <TableCell className="text-accent-brand-strong text-right text-xs font-medium tabular-nums">
                  {formatCost(s.total_cost_usd)}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

/** Fixed source options — the sources sessions are collected from. Brand
 *  names are stable, so they live here rather than in i18n (mirrors the usage
 *  view's source-labels); only the "all" option and labels are localized. */
const SOURCE_OPTIONS: readonly FilterOption[] = SOURCE_TAGS.map((src) => ({
  value: src,
  label: sessionSourceLabel(src),
}))
