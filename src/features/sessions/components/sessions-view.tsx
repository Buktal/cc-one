// Sessions view —— 三栏工作台容器（#108 定稿 variant-a「图标轨道 + 计数
// 清单」，原型 docs/prototype/sessions-v2/variant-a-icon-rail-counts.html）：
// 左栏（图标轨道条 + 纯计数清单）｜中内容区（分页会话列表 / 会话详情）｜
// 右统计卡栏（口径随选中对象派生：会话态四卡 / 项目态项目卡 / 分组态轻量
// 汇总）。顶部工具条：时间 pill（作用于全部统计）+ 筛选 + 搜索 + 批量操作
// 入口。
//
// 中内容区两态：列表态（未选会话）= 分页会话列表（每页 20/50/100）；会话态
// = 标题行 + 对话流（session-detail，其余统计全在右栏）。窄容器（< 48rem ≈
// 768 档）左树收成工具条下拉、右栏折叠为浮动按钮开抽屉。
//
// Pure rendering only — all state, queries, mutations and the optimistic
// favorite / pending-group handling live in useSessionsBrowser (./use-sessions-
// browser). This component owns JSX, styles, i18n and the source-display helper
// (../source-labels). Mirrors library-view.tsx's split.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { MessagesSquare, Search, Star, Trash2, X } from "lucide-react"
import { useTranslation } from "react-i18next"
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
import { SOURCE_TAGS } from "@/lib/source-tags"
import { cn } from "@/lib/utils"
import type { SessionRow } from "@/types/generated/bindings"
import { ALL_GROUPS, favKey, projectBasename, UNGROUPED } from "../derive"
import { highlight } from "../highlight"
import { sessionAgentKind, sessionSourceLabel } from "../source-labels"
import { PAGE_SIZES, useSessionsBrowser } from "../use-sessions-browser"
import { GroupCreateDialog } from "./group-create-dialog"
import { SessionDetail } from "./session-detail"
import { SessionTree } from "./session-tree"
import { StatsRail, type StatsScopeTag } from "./stats-rail"

dayjs.extend(relativeTime)

export function SessionsView() {
  const b = useSessionsBrowser()
  const { t } = useTranslation()
  // Narrow `preview` to a non-null local so the detail callbacks capture
  // a SessionRow, not SessionRow | null (TS will not narrow across callbacks
  // that read the field later).
  const preview = b.preview

  // 右栏口径对象名 + 会话粒度统计行（按会话卡的数据源）。selectedProject 用
  // != null 判空："" 是未知项目桶（哨兵映射后的 identity），真值语义。
  const scopeLabel = preview
    ? preview.title || t("sessions.untitled")
    : b.selectedProject != null
      ? projectBasename(b.selectedProject) || t("sessions.tree.noProject")
      : b.selectedGroupId === UNGROUPED
        ? t("sessions.group.ungrouped")
        : b.selectedGroupId === ALL_GROUPS
          ? t("sessions.stats.allSessions")
          : (b.trackGroups.find((g) => g.id === b.selectedGroupId)?.name ??
            t("sessions.stats.allSessions"))
  const sessionStats = preview
    ? (b.statsByKey.get(favKey(preview)) ?? null)
    : null
  // 右栏口径 tag 与项目态身份卡数据：口径由选中对象派生（会话 > 项目 > 分组），
  // 与 selectionAggregate 的容器判定同一优先级——tab 删除后不再有第二份手设。
  const groupSelected =
    b.selectedProject == null && b.selectedGroupId !== ALL_GROUPS
  const scopeTag: StatsScopeTag = preview
    ? "session"
    : groupSelected
      ? "group"
      : "project"
  const projectIdentity =
    !preview && b.selectedProject != null
      ? {
          dir: b.selectedProject,
          subagents: b.selectionRows.filter((r) => r.agent_type !== "").length,
        }
      : null

  return (
    // @container 驱动工作台自身的折叠（48rem ≈ 768 档树/右栏让位；64rem ≈
    // 1024 档右栏卡两列）——量的是内容区自身宽度，主导航折叠不牵动它。
    <div className="@container flex min-h-0 flex-1 flex-col gap-3">
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

        {/* 中内容区（两态：列表 / 会话详情）。@container 在此列——详情里的
            轮次导航列（TURN_NAV_VISIBILITY）以本列宽度显隐。 */}
        <div className="@container flex min-h-0 min-w-0 flex-1 flex-col">
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
            />
          ) : (
            <ListPane b={b} />
          )}
        </div>

        <StatsRail
          scopeTag={scopeTag}
          scopeLabel={scopeLabel}
          aggregate={b.selectionAggregate}
          session={preview}
          sessionStats={sessionStats}
          transcript={b.transcript}
          transcriptLoading={b.transcriptLoading}
          deviceLabel={(id) => b.deviceLabel.get(id) ?? id.slice(0, 8)}
          projectIdentity={projectIdentity}
        />
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

/** 顶部工具条：时间 pill + 筛选 + 搜索 + 批量操作；窄容器加树容器下拉。 */
function WorkbenchToolbar({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
  return (
    <div className="@container flex flex-wrap items-center gap-2">
      <div className="order-1 flex shrink-0 items-center gap-2">
        <DateRangeChip
          preset={b.rangePreset}
          fromDay={b.fromDay}
          toDay={b.toDay}
          onPreset={b.setRangePreset}
          onFromDay={b.setFromDay}
          onToDay={b.setToDay}
        />
        {/* 窄容器的树下拉（左树 < 48rem 让位）：列出当前轨道的容器。 */}
        <div className="@[48rem]:hidden">
          <NarrowTreeSelect b={b} />
        </div>
      </div>

      {/* Search rides line 1 (right-pinned); secondary filters wrap on line 2
          on narrow containers and inline before the search on wide ones. */}
      <div className="order-3 ml-auto flex shrink-0 items-center gap-2 @[60rem]:order-4">
        <BatchBar b={b} />
        <div className="relative w-56 @[48rem]:w-40 @[60rem]:w-56">
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
      <div className="order-4 flex w-full min-w-0 flex-wrap items-center gap-2 @[60rem]:order-3 @[60rem]:w-auto">
        <FilterSelect
          ariaLabel={t("sessions.filter.source")}
          allLabel={t("sessions.filter.allSources")}
          options={SOURCE_OPTIONS}
          value={b.source}
          onChange={b.setSource}
          className="h-8 w-30"
        />
        <FilterSelect
          ariaLabel={t("sessions.filter.model")}
          allLabel={t("sessions.filter.allModels")}
          options={b.modelOptions.map((m) => ({ value: m, label: m }))}
          value={b.model}
          onChange={b.setModel}
          className="h-8 w-40"
        />
        {/* 项目维度：共享 filterSlice（与看板 / 日志一致），左树项目轨道的
            选中也写同一份状态。候选取自 distinct-projects 端点，含「未知
            项目」特殊选项。 */}
        <ProjectSelect className="h-8 w-40" />
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
            className="h-8 w-30"
          />
        ) : null}
      </div>
    </div>
  )
}

/** 批量操作入口（定稿 §6）：勾选后出现——批量收藏 / 归组（下拉选组）/
 *  删除（会话删除属后续切片，入口就位但禁用）+ 清除多选。 */
function BatchBar({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
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
        className="h-8 w-32"
        triggerSize="sm"
      />
      <Tooltip>
        <TooltipTrigger
          render={
            <Button variant="outline" size="sm" disabled>
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
    </div>
  )
}

/** 窄容器的容器下拉：全部 + 当前轨道的容器（项目 basename / 组名）。值编码
 *  "p:<dir>" / "g:<id>"，onChange 解码回选中动作。项目 identity 为 ""（未知
 *  项目桶）时编码为 "p:"——用 != null 区分「未选」与「空桶」。 */
function NarrowTreeSelect({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
  const options: FilterOption[] =
    b.track === "projects"
      ? b.projectBuckets.map((n) => ({
          value: `p:${n.project}`,
          label: projectBasename(n.project) || t("sessions.tree.noProject"),
        }))
      : b.trackGroups.map((g) => ({ value: `g:${g.id}`, label: g.name }))
  const value =
    b.selectedProject != null
      ? `p:${b.selectedProject}`
      : b.selectedGroupId === UNGROUPED
        ? `g:${UNGROUPED}`
        : b.selectedGroupId === ALL_GROUPS
          ? ""
          : `g:${b.selectedGroupId}`
  return (
    <FilterSelect
      ariaLabel={t("sessions.tree.all")}
      allLabel={t("sessions.tree.all")}
      options={options}
      value={value}
      onChange={(v) => {
        if (!v) b.selectAll()
        else if (v.startsWith("p:")) b.selectProject(v.slice(2))
        else b.selectGroup(v.slice(2))
      }}
      className="h-8 w-40"
      fallbackLabel={t("sessions.tree.all")}
    />
  )
}

// ------------------------------------------------------------- 中栏 ----

/** 列表态骨架：头部（标题 + 项目路径 + 会话数）+ 表格 + 分页（每页 20/50/
 *  100）。selectedProject 以 != null 判选中："" = 未知项目桶。项目统计头
 *  已上移右栏（项目态项目卡），中栏只管列表。 */
function ListPane({ b }: { b: ReturnType<typeof useSessionsBrowser> }) {
  const { t } = useTranslation()
  const headTitle =
    b.selectedProject != null
      ? projectBasename(b.selectedProject) || t("sessions.tree.noProject")
      : b.selectedGroupId === UNGROUPED
        ? t("sessions.group.ungrouped")
        : b.selectedGroupId === ALL_GROUPS
          ? t("sessions.tree.all")
          : (b.trackGroups.find((g) => g.id === b.selectedGroupId)?.name ??
            t("sessions.tree.all"))
  const headDesc = b.selectedProject ?? ""

  return (
    <Card className="flex min-h-0 min-w-0 flex-1 flex-col">
      <CardContent className="flex min-h-0 flex-1 flex-col gap-2 p-4">
        <div className="flex items-baseline gap-2.5 px-0.5">
          <h3 className="text-sm font-semibold">{headTitle}</h3>
          {headDesc ? (
            <span className="text-muted-foreground min-w-0 flex-1 truncate text-xs">
              {headDesc}
            </span>
          ) : null}
          {/* DSL 段：会话数 N（与分页条同源的 viewTotal）。 */}
          <span className="text-muted-foreground ml-auto shrink-0 text-xs tabular-nums">
            {formatMetricSeg(
              t("sessions.stats.sessions"),
              formatCount(b.viewTotal),
            )}
          </span>
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
}) {
  const { t } = useTranslation()
  return (
    <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
      {/* table-fixed: column widths come from the header row, so the narrow
          numeric columns are never stretched by extra horizontal space. min-w
          keeps the title readable below the fixed sum — the outer overflow-auto
          scrolls horizontally instead of squeezing columns into overlap. */}
      <Table className="table-fixed min-w-[58rem]">
        <TableHeader>
          <TableRow>
            {/* 批量勾选列（定稿 §6 批量操作入口）。 */}
            <TableHead className="w-9" />
            <TableHead className="w-9" />
            <TableHead className="max-w-[24rem]">
              {t("sessions.col.title")}
            </TableHead>
            <TableHead className="w-36">{t("sessions.col.type")}</TableHead>
            {showDeviceColumn ? (
              <TableHead className="w-28">{t("sessions.col.device")}</TableHead>
            ) : null}
            {showProjectColumn ? (
              <TableHead className="w-44">
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
            return (
              <TableRow
                key={favKey(s)}
                // Selected row keeps its tint on hover too — the default
                // hover:bg-hover would otherwise flash grey over it.
                className={cn(open && "bg-accent-tint hover:bg-accent-tint")}
              >
                <TableCell>
                  <Checkbox
                    checked={isChecked(s)}
                    onCheckedChange={() => onToggleCheck(s)}
                    aria-label={t("sessions.batch.check", {
                      title: s.title || t("sessions.untitled"),
                    })}
                  />
                </TableCell>
                <TableCell>
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
                          <span className="text-muted-foreground/50 mr-0.5">
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
