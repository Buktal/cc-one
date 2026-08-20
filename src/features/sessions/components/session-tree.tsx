// 左栏 —— 三栏工作台第一栏（#108 定稿 variant-a「图标轨道 + 计数清单」）。
// 三轨道收成左栏顶部图标轨道条（项目 = 自动桶 / 分组 = 本机手动组 / 收藏 =
// 同步组 × 跨设备收藏宇宙），轨道即页面的宇宙开关；下方是纯计数清单——
// 节点只显示「名称 + 会话数胶囊」，不再展开会话子行（要看会话去中栏列表）。
// 计数与中栏列表同源：都由 useSessionsBrowser 的 selection-free session_stats
// 读派生，时间/来源/模型/设备筛选与搜索实时联动；容器选中不改计数（facet
// 规则——选中 A 组时 B/C/D 的数字不该归零）。subagent 会话按普通行计入其
// 项目桶（#84 已把 worktree 会话归父项目），挂主会话的缩进展示待 #90。
//
// 纯渲染 —— 轨道/选中之外的态都在 useSessionsBrowser；分组节点的 CRUD 弹层
// 复用 group-sidebar 的 GroupActionsPopover，拖拽排序沿用 dnd-kit 距离约束
// （6px 内仍是点击）。

import { PointerActivationConstraints } from "@dnd-kit/dom"
import {
  DragDropProvider,
  type DragEndEvent,
  PointerSensor,
} from "@dnd-kit/react"
import { useSortable } from "@dnd-kit/react/sortable"
import { Box, Loader2, Plus, Star, Tag } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatCount } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SessionGroup, SessionStatsRow } from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  type ProjectNodeData,
  projectBasename,
  reorderGroupIds,
  type TreeTrack,
  UNGROUPED,
} from "../derive"
import { GroupActionsPopover } from "./group-sidebar"

export function SessionTree({
  track,
  onTrackChange,
  statsRows,
  projectBuckets,
  groupBuckets,
  trackGroups,
  selectedGroupId,
  selectedProject,
  onSelectAll,
  onSelectProject,
  onSelectGroup,
  // group CRUD (分组/收藏轨)
  onCreateGroup,
  onRenameGroup,
  onDeleteGroup,
  onReorderGroups,
  pendingGroup,
  busyGroupId,
}: {
  track: TreeTrack
  onTrackChange: (t: TreeTrack) => void
  /** Selection-free 宇宙统计行——「全部」行计数的来源（与各桶计数同一次读）。 */
  statsRows: SessionStatsRow[]
  projectBuckets: ProjectNodeData[]
  groupBuckets: {
    grouped: Map<string, SessionStatsRow[]>
    ungrouped: SessionStatsRow[]
  }
  trackGroups: SessionGroup[]
  selectedGroupId: string
  selectedProject: string | null
  onSelectAll: () => void
  onSelectProject: (project: string) => void
  onSelectGroup: (groupId: string) => void
  onCreateGroup: () => void
  onRenameGroup: (g: SessionGroup, name: string) => Promise<void>
  onDeleteGroup: (g: SessionGroup) => Promise<void>
  onReorderGroups: (orderedIds: string[]) => void
  pendingGroup: string | null
  busyGroupId: string | null
}) {
  const { t } = useTranslation()
  const nothingSelected =
    selectedGroupId === ALL_GROUPS && selectedProject == null

  return (
    <div className="border-border bg-card hidden min-h-0 w-60 shrink-0 flex-col rounded-lg border @[48rem]:flex">
      {/* 图标轨道条：三轨宇宙开关（图标 + 小字竖排）。 */}
      <div className="border-border flex items-center gap-1 border-b p-2">
        <Tabs
          value={track}
          onValueChange={(v) => onTrackChange(v as TreeTrack)}
          className="min-w-0 flex-1"
        >
          {/* h-auto 覆盖（同变体）：TabsList 横向默认 h-8 / Trigger 默认
              calc(100%-1px)——图标竖排触发器需要按内容撑高。 */}
          <TabsList className="grid-cols-3 group-data-horizontal/tabs:h-auto w-full">
            <TrackTrigger
              value="projects"
              icon={Box}
              label={t("sessions.tree.track.projects")}
            />
            <TrackTrigger
              value="groups"
              icon={Tag}
              label={t("sessions.tree.track.groups")}
            />
            <TrackTrigger
              value="favorites"
              icon={Star}
              label={t("sessions.tree.track.favorites")}
            />
          </TabsList>
        </Tabs>
        {track !== "projects" ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={t("sessions.group.create")}
                  onClick={onCreateGroup}
                  disabled={pendingGroup !== null}
                  className="shrink-0"
                />
              }
            >
              <Plus />
            </TooltipTrigger>
            <TooltipContent>{t("sessions.group.create")}</TooltipContent>
          </Tooltip>
        ) : null}
      </div>

      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-0.5 p-2">
          <CountRow
            icon={Box}
            label={t("sessions.tree.all")}
            count={statsRows.length}
            selected={nothingSelected}
            onClick={onSelectAll}
          />

          <div className="bg-border mx-2 my-1.5 h-px" />

          {track === "projects"
            ? projectBuckets.map((node) => (
                <CountRow
                  key={node.project}
                  icon={Box}
                  label={
                    projectBasename(node.project) ||
                    t("sessions.tree.noProject")
                  }
                  count={node.sessions.length}
                  selected={selectedProject === node.project}
                  tooltip={node.project || t("sessions.tree.noProject")}
                  onClick={() => onSelectProject(node.project)}
                />
              ))
            : null}

          {track !== "projects" ? (
            <GroupTrack
              trackGroups={trackGroups}
              groupBuckets={groupBuckets}
              selectedGroupId={selectedGroupId}
              onSelect={onSelectGroup}
              onRenameGroup={onRenameGroup}
              onDeleteGroup={onDeleteGroup}
              onReorderGroups={onReorderGroups}
              pendingGroup={pendingGroup}
              busyGroupId={busyGroupId}
            />
          ) : null}
        </div>
      </ScrollArea>

      <div className="border-border text-muted-foreground/60 border-t px-3 py-1.5 text-[10px] whitespace-nowrap">
        {t("sessions.tree.hint")}
      </div>
    </div>
  )
}

/** 轨道条触发器：图标 + 小字竖排（去掉 TabsTrigger 默认的下划线指示条，
 *  选中态走品牌浅色面）。 */
function TrackTrigger({
  value,
  icon: Icon,
  label,
}: {
  value: TreeTrack
  icon: typeof Box
  label: string
}) {
  return (
    <TabsTrigger
      value={value}
      className="after:hidden h-auto flex-col gap-1 py-1.5 text-[10px] leading-none"
    >
      <Icon className="size-3.5" />
      {label}
    </TabsTrigger>
  )
}

/** 会话数胶囊 —— 计数清单行的唯一计量（DSL 计数类）。选中 = 主面填充，
 *  与全站选中态同向。 */
function CountPill({ count, selected }: { count: number; selected: boolean }) {
  return (
    <span
      className={cn(
        "min-w-[30px] shrink-0 rounded-full px-2 py-0.5 text-center text-[11px] tabular-nums",
        selected
          ? "bg-primary font-semibold text-primary-foreground"
          : "bg-muted text-muted-foreground",
      )}
    >
      {formatCount(count)}
    </span>
  )
}

/** 行内容（图标 + 名称 + 计数胶囊）——计数清单所有节点共享的唯一实现。 */
function CountRowInner({
  icon: Icon,
  label,
  count,
  selected,
}: {
  icon: typeof Box
  label: string
  count: number
  selected: boolean
}) {
  return (
    <>
      <Icon className="text-muted-foreground size-3.5 shrink-0" />
      <span
        className={cn(
          "min-w-0 flex-1 truncate",
          selected && "text-accent-brand-strong font-medium",
        )}
      >
        {label}
      </span>
      <CountPill count={count} selected={selected} />
    </>
  )
}

/** 计数清单的普通行（全部 / 项目 / 未分组）：整行即按钮；项目行带全路径
 *  tooltip。 */
function CountRow({
  icon,
  label,
  count,
  selected,
  tooltip,
  onClick,
}: {
  icon: typeof Box
  label: string
  count: number
  selected: boolean
  /** 悬停补充（项目节点的完整路径）；缺省不包 Tooltip。 */
  tooltip?: string
  onClick: () => void
}) {
  const button = (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "hover:bg-hover flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] transition-colors",
        selected ? "bg-accent-tint hover:bg-accent-tint" : "text-foreground",
      )}
    >
      <CountRowInner
        icon={icon}
        label={label}
        count={count}
        selected={selected}
      />
    </button>
  )
  if (!tooltip) return button
  return (
    <Tooltip>
      <TooltipTrigger render={button} />
      <TooltipContent side="right" className="max-w-sm break-all">
        {tooltip}
      </TooltipContent>
    </Tooltip>
  )
}

/** 分组/收藏轨：拖拽排序的组节点 + 未分组桶 + 进行中的乐观行。 */
function GroupTrack({
  trackGroups,
  groupBuckets,
  selectedGroupId,
  onSelect,
  onRenameGroup,
  onDeleteGroup,
  onReorderGroups,
  pendingGroup,
  busyGroupId,
}: {
  trackGroups: SessionGroup[]
  groupBuckets: {
    grouped: Map<string, SessionStatsRow[]>
    ungrouped: SessionStatsRow[]
  }
  selectedGroupId: string
  onSelect: (groupId: string) => void
  onRenameGroup: (g: SessionGroup, name: string) => Promise<void>
  onDeleteGroup: (g: SessionGroup) => Promise<void>
  onReorderGroups: (orderedIds: string[]) => void
  pendingGroup: string | null
  busyGroupId: string | null
}) {
  const { t } = useTranslation()
  // Whole-row drag handle: 6px of movement before a press becomes a drag —
  // clicks keep selecting the row / opening its popover; moves reorder.
  const sensors = [
    PointerSensor.configure({
      activationConstraints: () => [
        new PointerActivationConstraints.Distance({ value: 6 }),
      ],
      preventActivation: () => false,
    }),
  ]
  function handleDragEnd(event: DragEndEvent): void {
    if (event.canceled) return
    const sourceId = event.operation.source?.id
    const targetId = event.operation.target?.id
    if (sourceId == null || targetId == null || sourceId === targetId) return
    const next = reorderGroupIds(
      trackGroups.map((g) => g.id),
      String(sourceId),
      String(targetId),
    )
    if (next) onReorderGroups(next)
  }
  return (
    <>
      <DragDropProvider sensors={sensors} onDragEnd={handleDragEnd}>
        {trackGroups.map((g, i) => (
          <GroupNodeRow
            key={g.id}
            group={g}
            index={i}
            count={groupBuckets.grouped.get(g.id)?.length ?? 0}
            selected={selectedGroupId === g.id}
            onSelect={() => onSelect(g.id)}
            onRename={onRenameGroup}
            onDelete={onDeleteGroup}
            busy={busyGroupId === g.id}
          />
        ))}
      </DragDropProvider>
      {pendingGroup ? (
        <div className="text-muted-foreground flex items-center gap-2 rounded-md px-2 py-1.5 text-[13px]">
          <Loader2 className="size-3.5 shrink-0 animate-spin" />
          <span className="truncate">{pendingGroup}</span>
        </div>
      ) : null}
      <CountRow
        icon={Tag}
        label={t("sessions.group.ungrouped")}
        count={groupBuckets.ungrouped.length}
        selected={selectedGroupId === UNGROUPED}
        onClick={() => onSelect(UNGROUPED)}
      />
    </>
  )
}

/** 组节点行：可拖拽整行 + ⋮ CRUD 弹层（复用 GroupActionsPopover）。 */
function GroupNodeRow({
  group: g,
  index,
  count,
  selected,
  onSelect,
  onRename,
  onDelete,
  busy,
}: {
  group: SessionGroup
  index: number
  count: number
  selected: boolean
  onSelect: () => void
  onRename: (g: SessionGroup, name: string) => Promise<void>
  onDelete: (g: SessionGroup) => Promise<void>
  busy: boolean
}) {
  const { ref, isDragging } = useSortable({
    id: g.id,
    index,
    disabled: busy,
  })
  return (
    <div
      ref={ref}
      className={cn(
        "group/grow hover:bg-hover flex items-center gap-1 rounded-md transition-colors",
        selected && "bg-accent-tint hover:bg-accent-tint",
        busy && "opacity-60",
        isDragging && "opacity-60 shadow-sm",
      )}
    >
      <button
        type="button"
        className={cn(
          "flex min-w-0 flex-1 items-center gap-2 rounded-md py-1.5 pr-1 pl-2 text-left text-[13px]",
          selected ? "text-accent-brand-strong" : "text-foreground",
        )}
        onClick={onSelect}
        disabled={busy}
      >
        <CountRowInner
          icon={Tag}
          label={g.name}
          count={count}
          selected={selected}
        />
      </button>
      {/* ⋮ 悬停展开：零宽起步，hover/focus 展开（与旧侧栏同一手法）。 */}
      <div className="w-0 overflow-hidden pr-0.5 transition-[width] duration-150 ease-out group-hover/grow:w-6 group-focus-within/grow:w-6">
        <GroupActionsPopover
          group={g}
          onRename={onRename}
          onDelete={onDelete}
          busy={busy}
        />
      </div>
    </div>
  )
}
