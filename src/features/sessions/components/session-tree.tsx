// 左树栏 —— 四栏工作台第二栏（定稿 docs/plans/sessions-workbench-redesign.md
// §1）。三轨道并存：项目（自动桶）/ 分组（本机手动组）/ 收藏（同步组 × 跨设备
// 收藏宇宙）；轨道即页面的宇宙开关（收藏轨 = 旧 Favorites tab 语义）。
//
// 两级树：容器 → 会话子行。每容器默认前 3 条 + 「还有 N 条」展开（防单个大
// 项目吞列）；选中容器自动展开一次（用户仍可手动收起）。项目节点带小统计
// （会话数 · Token，#85 同源的 session_stats 读）。subagent 会话在子行带
// agent 类型徽章——数据层暂无「主会话」关联字段，子行按普通行归属其项目桶
// （#84 已把 worktree 会话归父项目），挂主会话下的缩进待数据层补父链接后落地。
//
// 纯渲染 —— 轨道/选中/展开溢出之外的态都在 useSessionsBrowser；分组节点的
// CRUD 弹层复用 group-sidebar 的 GroupActionsPopover，拖拽排序沿用 dnd-kit
// 距离约束（6px 内仍是点击）。

import { PointerActivationConstraints } from "@dnd-kit/dom"
import {
  DragDropProvider,
  type DragEndEvent,
  PointerSensor,
} from "@dnd-kit/react"
import { useSortable } from "@dnd-kit/react/sortable"
import { Box, ChevronRight, Loader2, Plus, Star, Tag } from "lucide-react"
import dayjs from "dayjs"
import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"
import type {
  SessionGroup,
  SessionStatsRow,
} from "@/types/generated/bindings"
import {
  aggregateStats,
  ALL_GROUPS,
  favKey,
  projectBasename,
  type ProjectNodeData,
  reorderGroupIds,
  type TreeTrack,
  UNGROUPED,
} from "../derive"
import { GroupActionsPopover } from "./group-sidebar"

/** 每容器默认展示的会话子行数（定稿 §1：前 3 条 + 展开更多）。 */
const PREVIEW_ROWS = 3

export function SessionTree({
  track,
  onTrackChange,
  statsRows,
  projectBuckets,
  groupBuckets,
  trackGroups,
  selectedGroupId,
  selectedProject,
  activeSessionKey,
  onSelectAll,
  onSelectProject,
  onSelectGroup,
  onOpenSession,
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
  /** Selection-free 宇宙统计行（后端已按 last_active 倒序）。 */
  statsRows: SessionStatsRow[]
  projectBuckets: ProjectNodeData[]
  groupBuckets: {
    grouped: Map<string, SessionStatsRow[]>
    ungrouped: SessionStatsRow[]
  }
  trackGroups: SessionGroup[]
  selectedGroupId: string
  selectedProject: string | null
  /** 当前详情会话的 favKey —— 对应子行高亮。 */
  activeSessionKey: string | null
  onSelectAll: () => void
  onSelectProject: (project: string) => void
  onSelectGroup: (groupId: string) => void
  onOpenSession: (row: SessionStatsRow) => void
  onCreateGroup: () => void
  onRenameGroup: (g: SessionGroup, name: string) => Promise<void>
  onDeleteGroup: (g: SessionGroup) => Promise<void>
  onReorderGroups: (orderedIds: string[]) => void
  pendingGroup: string | null
  busyGroupId: string | null
}) {
  const { t } = useTranslation()
  // 展开 / 溢出都是节点的瞬时 UI 态：键 = "p:<dir>" / "g:<id>"。选中变化把
  // 选中节点加进展开集（一次性自动展开，用户仍可收起）。
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const [overflowOpen, setOverflowOpen] = useState<Set<string>>(() => new Set())
  const selectedKey =
    selectedProject != null ? `p:${selectedProject}` : `g:${selectedGroupId}`
  useEffect(() => {
    if (selectedGroupId === ALL_GROUPS && selectedProject == null) return
    setExpanded((prev) =>
      prev.has(selectedKey) ? prev : new Set(prev).add(selectedKey),
    )
  }, [selectedKey, selectedGroupId, selectedProject])

  const nothingSelected = selectedGroupId === ALL_GROUPS && selectedProject == null
  const universe = useMemo(() => aggregateStats(statsRows), [statsRows])

  function toggleNode(key: string): void {
    setExpanded((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }
  function openOverflow(key: string): void {
    setOverflowOpen((prev) => new Set(prev).add(key))
  }

  /** 容器节点体：头行 + （展开时）子行 + 「还有 N 条」。 */
  function nodeBlock(
    key: string,
    head: React.ReactNode,
    rows: SessionStatsRow[],
  ) {
    const open = expanded.has(key)
    const all = overflowOpen.has(key)
    const shown = open ? (all ? rows : rows.slice(0, PREVIEW_ROWS)) : []
    const rest = open && !all ? Math.max(0, rows.length - PREVIEW_ROWS) : 0
    return (
      <div key={key}>
        {head}
        {shown.map((r) => (
          <SessionChildRow
            key={favKey(r)}
            row={r}
            active={favKey(r) === activeSessionKey}
            onOpen={onOpenSession}
          />
        ))}
        {rest > 0 ? (
          <button
            type="button"
            onClick={() => openOverflow(key)}
            className="text-muted-foreground hover:bg-hover hover:text-foreground mt-0.5 ml-[30px] block w-[calc(100%-36px)] rounded-md px-2 py-1 text-left text-[11px]"
          >
            {t("sessions.tree.more", { n: rest })}
          </button>
        ) : null}
      </div>
    )
  }

  return (
    <div className="border-border bg-card hidden min-h-0 w-60 shrink-0 flex-col gap-1 rounded-lg border p-2 @[48rem]:flex">
      {/* 轨道行：三轨 tab；分组/收藏轨带建组入口（+ 钉在轨道行右侧）。 */}
      <div className="flex items-center gap-1 pr-0.5 pl-0.5">
        <Tabs
          value={track}
          onValueChange={(v) => onTrackChange(v as TreeTrack)}
          className="min-w-0 flex-1"
        >
          <TabsList className="grid w-full grid-cols-3">
            <TabsTrigger value="projects" className="px-1 text-xs">
              {t("sessions.tree.track.projects")}
            </TabsTrigger>
            <TabsTrigger value="groups" className="px-1 text-xs">
              {t("sessions.tree.track.groups")}
            </TabsTrigger>
            <TabsTrigger value="favorites" className="px-1 text-xs">
              {t("sessions.tree.track.favorites")}
            </TabsTrigger>
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
        <div className="flex flex-col gap-0.5 pr-1">
          <button
            type="button"
            onClick={onSelectAll}
            className={cn(
              "hover:bg-hover flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
              nothingSelected &&
                "bg-accent-tint text-accent-brand-strong hover:bg-accent-tint",
              !nothingSelected && "text-muted-foreground",
            )}
          >
            <span className="flex-1 truncate">{t("sessions.tree.all")}</span>
            <span className="text-muted-foreground/70 text-[11px] tabular-nums">
              {statsRows.length} · {formatTokens(bucketTotal(universe))}
            </span>
          </button>

          <div className="bg-border mx-2 my-1.5 h-px" />

          {track === "projects"
            ? projectBuckets.map((node) =>
                nodeBlock(
                  `p:${node.project}`,
                  <ProjectNodeRow
                    node={node}
                    selected={selectedProject === node.project}
                    open={expanded.has(`p:${node.project}`)}
                    onToggle={() => toggleNode(`p:${node.project}`)}
                    onSelect={onSelectProject}
                  />,
                  node.sessions,
                ),
              )
            : null}

          {track !== "projects" ? (
            <GroupTrack
              trackGroups={trackGroups}
              groupBuckets={groupBuckets}
              selectedGroupId={selectedGroupId}
              expanded={expanded}
              onToggle={toggleNode}
              onSelect={onSelectGroup}
              onRenameGroup={onRenameGroup}
              onDeleteGroup={onDeleteGroup}
              onReorderGroups={onReorderGroups}
              pendingGroup={pendingGroup}
              busyGroupId={busyGroupId}
              nodeBlock={nodeBlock}
            />
          ) : null}
        </div>
      </ScrollArea>
    </div>
  )
}

/** 四桶求和 —— 树节点「Token」小统计的显示值。 */
function bucketTotal(a: { tokens: ReturnType<typeof aggregateStats>["tokens"] }) {
  return (
    a.tokens.input +
    a.tokens.output +
    a.tokens.cache_creation +
    a.tokens.cache_read
  )
}

/** 项目节点头行：basename + 悬停全路径（定稿 §痛点1）+ 会话数 · Token。 */
function ProjectNodeRow({
  node,
  selected,
  open,
  onToggle,
  onSelect,
}: {
  node: ProjectNodeData
  selected: boolean
  open: boolean
  onToggle: () => void
  onSelect: (project: string) => void
}) {
  const { t } = useTranslation()
  const name = node.project ? projectBasename(node.project) : ""
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            onClick={() => onSelect(node.project)}
            className={cn(
              "hover:bg-hover flex w-full items-center gap-1.5 rounded-md px-1.5 py-1.5 text-left text-sm transition-colors",
              selected
                ? "bg-accent-tint text-accent-brand-strong hover:bg-accent-tint"
                : "text-foreground",
            )}
          />
        }
      >
        <ChevronRight
          className={cn(
            "text-muted-foreground/60 size-3.5 shrink-0 transition-transform",
            open && "rotate-90",
          )}
          onClick={(e) => {
            e.stopPropagation()
            onToggle()
          }}
        />
        <Box className="text-muted-foreground size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate text-[13px] font-medium">
          {name || t("sessions.tree.noProject")}
        </span>
        <span className="text-muted-foreground/70 shrink-0 text-[11px] tabular-nums">
          {node.sessions.length} · {formatTokens(node.tokens)}
        </span>
      </TooltipTrigger>
      <TooltipContent side="right" className="max-w-sm break-all">
        {node.project || t("sessions.tree.noProject")}
      </TooltipContent>
    </Tooltip>
  )
}

/** 分组/收藏轨：拖拽排序的组节点 + 未分组桶 + 进行中的乐观行。 */
function GroupTrack({
  trackGroups,
  groupBuckets,
  selectedGroupId,
  expanded,
  onToggle,
  onSelect,
  onRenameGroup,
  onDeleteGroup,
  onReorderGroups,
  pendingGroup,
  busyGroupId,
  nodeBlock,
}: {
  trackGroups: SessionGroup[]
  groupBuckets: {
    grouped: Map<string, SessionStatsRow[]>
    ungrouped: SessionStatsRow[]
  }
  selectedGroupId: string
  expanded: ReadonlySet<string>
  onToggle: (key: string) => void
  onSelect: (groupId: string) => void
  onRenameGroup: (g: SessionGroup, name: string) => Promise<void>
  onDeleteGroup: (g: SessionGroup) => Promise<void>
  onReorderGroups: (orderedIds: string[]) => void
  pendingGroup: string | null
  busyGroupId: string | null
  nodeBlock: (
    key: string,
    head: React.ReactNode,
    rows: SessionStatsRow[],
  ) => React.ReactNode
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
        {trackGroups.map((g, i) =>
          nodeBlock(
            `g:${g.id}`,
            <GroupNodeRow
              key={g.id}
              group={g}
              index={i}
              count={groupBuckets.grouped.get(g.id)?.length ?? 0}
              selected={selectedGroupId === g.id}
              open={expanded.has(`g:${g.id}`)}
              onToggle={() => onToggle(`g:${g.id}`)}
              onSelect={() => onSelect(g.id)}
              onRename={onRenameGroup}
              onDelete={onDeleteGroup}
              busy={busyGroupId === g.id}
            />,
            groupBuckets.grouped.get(g.id) ?? [],
          ),
        )}
      </DragDropProvider>
      {pendingGroup ? (
        <div className="text-muted-foreground flex items-center gap-2 rounded-md px-2 py-1.5 text-sm">
          <Loader2 className="size-3.5 shrink-0 animate-spin" />
          <span className="truncate">{pendingGroup}</span>
        </div>
      ) : null}
      {nodeBlock(
        `g:${UNGROUPED}`,
        <PlainGroupRow
          label={t("sessions.group.ungrouped")}
          count={groupBuckets.ungrouped.length}
          selected={selectedGroupId === UNGROUPED}
          onSelect={() => onSelect(UNGROUPED)}
          open={expanded.has(`g:${UNGROUPED}`)}
          onToggle={() => onToggle(`g:${UNGROUPED}`)}
        />,
        groupBuckets.ungrouped,
      )}
    </>
  )
}

function PlainGroupRow({
  label,
  count,
  selected,
  open,
  onToggle,
  onSelect,
}: {
  label: string
  count: number
  selected: boolean
  open: boolean
  onToggle: () => void
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={cn(
        "hover:bg-hover flex w-full items-center gap-1.5 rounded-md px-1.5 py-1.5 text-left text-sm transition-colors",
        selected
          ? "bg-accent-tint text-accent-brand-strong hover:bg-accent-tint"
          : "text-muted-foreground",
      )}
    >
      <ChevronRight
        className={cn(
          "text-muted-foreground/60 size-3.5 shrink-0 transition-transform",
          open && "rotate-90",
        )}
        onClick={(e) => {
          e.stopPropagation()
          onToggle()
        }}
      />
      <Tag className="size-3.5 shrink-0" />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="text-muted-foreground/70 shrink-0 text-[11px] tabular-nums">
        {count}
      </span>
    </button>
  )
}

/** 组节点头行：可拖拽整行 + ⋮ CRUD 弹层（复用 GroupActionsPopover）。 */
function GroupNodeRow({
  group: g,
  index,
  count,
  selected,
  open,
  onToggle,
  onSelect,
  onRename,
  onDelete,
  busy,
}: {
  group: SessionGroup
  index: number
  count: number
  selected: boolean
  open: boolean
  onToggle: () => void
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
        "group/grow hover:bg-hover flex items-center gap-1 rounded-md px-1.5 py-1.5 text-sm transition-colors",
        selected
          ? "bg-accent-tint text-accent-brand-strong hover:bg-accent-tint"
          : "text-muted-foreground",
        busy && "opacity-60",
        isDragging && "opacity-60 shadow-sm",
      )}
    >
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-1.5 text-left"
        onClick={onSelect}
        disabled={busy}
      >
        <ChevronRight
          className={cn(
            "text-muted-foreground/60 size-3.5 shrink-0 transition-transform",
            open && "rotate-90",
          )}
          onClick={(e) => {
            e.stopPropagation()
            onToggle()
          }}
        />
        <Tag className="size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate">{g.name}</span>
      </button>
      <span className="text-muted-foreground/70 text-[11px] tabular-nums">
        {count}
      </span>
      {/* ⋮ 悬停展开：零宽起步，hover/focus 展开（与旧侧栏同一手法）。 */}
      <div className="w-0 overflow-hidden transition-[width] duration-150 ease-out group-hover/grow:w-6 group-focus-within/grow:w-6">
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

/** 会话子行：缩进标题 + 收藏星 + 时间；subagent 加深缩进并带类型徽章。 */
function SessionChildRow({
  row,
  active,
  onOpen,
}: {
  row: SessionStatsRow
  active: boolean
  onOpen: (row: SessionStatsRow) => void
}) {
  const { t } = useTranslation()
  const sub = row.agent_type !== ""
  const time = row.last_active_at
    ? dayjs(row.last_active_at).isSame(dayjs(), "day")
      ? dayjs(row.last_active_at).format("HH:mm")
      : dayjs(row.last_active_at).format("MM-DD")
    : ""
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            onClick={() => onOpen(row)}
            className={cn(
              "hover:bg-hover flex w-full min-w-0 items-center gap-1.5 rounded-md py-1 pr-1.5 text-left text-xs transition-colors",
              sub ? "pl-[38px]" : "pl-[22px]",
              active
                ? "bg-accent-tint text-foreground"
                : "text-muted-foreground hover:text-foreground",
            )}
          />
        }
      >
        <span className="min-w-0 flex-1 truncate">
          {sub ? (
            <span className="text-muted-foreground/50 mr-0.5">↳</span>
          ) : null}
          {row.title || t("sessions.untitled")}
        </span>
        {row.favorited && !sub ? (
          <Star className="fill-accent-brand text-accent-brand size-3 shrink-0" />
        ) : null}
        {sub ? (
          <span className="sem-chip type-sub max-w-16 shrink-0">
            <span className="min-w-0 truncate">{row.agent_type}</span>
          </span>
        ) : null}
        <span className="text-muted-foreground/60 shrink-0 text-[10px] tabular-nums">
          {time}
        </span>
      </TooltipTrigger>
      <TooltipContent side="right" className="max-w-md">
        {row.title || t("sessions.untitled")}
      </TooltipContent>
    </Tooltip>
  )
}
