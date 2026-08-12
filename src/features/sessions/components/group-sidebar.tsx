// Group sidebar — the per-track group switcher for the sessions list. Renders
// "All", one row per group (with a live session count + a popover for rename /
// delete), "Ungrouped", an optimistic pending row for an in-flight synced-group
// create, and a "+ New group" button. Local groups rename/delete immediately;
// synced groups round-trip through git so their row shows a spinner while busy.
//
// Pure rendering — selection state, CRUD handlers, and the pending/busy flags
// come from useSessionsBrowser. The rename input inside each row's popover is
// transient local state (the hook only learns the new name on submit).

import { PointerActivationConstraints } from "@dnd-kit/dom"
import {
  DragDropProvider,
  type DragEndEvent,
  PointerSensor,
} from "@dnd-kit/react"
import { useSortable } from "@dnd-kit/react/sortable"
import {
  Check,
  Loader2,
  MoreHorizontal,
  Pencil,
  Plus,
  Trash2,
  X,
} from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"
import type { SessionGroup } from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  type GroupTrack,
  reorderGroupIds,
  UNGROUPED,
} from "../derive"

export function GroupSidebar({
  trackGroups,
  groupCounts,
  ungroupedCount,
  totalCount,
  selectedGroupId,
  onSelect,
  onCreate,
  onRename,
  onDelete,
  onReorder,
  pendingGroup,
  busyGroupId,
  track,
}: {
  trackGroups: SessionGroup[]
  /** Per-bucket session counts under the current filter (backend aggregation). */
  groupCounts: Map<string, number>
  /** Derived ungrouped count — total minus the known-group buckets. */
  ungroupedCount: number
  totalCount: number
  selectedGroupId: string
  onSelect: (id: string) => void
  onCreate: () => void
  onRename: (g: SessionGroup, name: string) => Promise<void>
  onDelete: (g: SessionGroup) => Promise<void>
  onReorder: (orderedIds: string[]) => void
  pendingGroup: string | null
  busyGroupId: string | null
  track: GroupTrack
}) {
  const { t } = useTranslation()
  const countById = groupCounts
  // Whole-row drag handle: 6px of movement before a press becomes a drag —
  // clicks keep selecting the row / opening its popover; moves reorder.
  // preventActivation is disabled so a press on the row's inner <button> can
  // start a drag too; dnd-kit suppresses the click once a drag actually
  // activates (the distance constraint keeps plain clicks intact).
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
    if (next) onReorder(next)
  }

  return (
    <div className="border-border bg-card flex min-h-0 w-44 shrink-0 flex-col gap-1 rounded-lg border p-2">
      {/* Title row with the create entry point — the + sits beside the track
        title (not under the list) so it stays visible and the group list keeps
        the full sidebar height. */}
      <div className="flex items-center justify-between pr-0.5 pl-1.5">
        <span className="text-muted-foreground py-0.5 text-xs font-medium">
          {track === "local"
            ? t("sessions.group.localTitle")
            : t("sessions.group.syncedTitle")}
        </span>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={t("sessions.group.create")}
                onClick={onCreate}
                disabled={pendingGroup !== null}
              />
            }
          >
            <Plus />
          </TooltipTrigger>
          <TooltipContent>{t("sessions.group.create")}</TooltipContent>
        </Tooltip>
      </div>
      {/* min-h-0: without it the ScrollArea grows with its content, stretching
        the whole sidebar once the group list gets long. Mirrors the right
        Card's `flex min-h-0 flex-1` pattern. */}
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-0.5 pr-1">
          <SidebarItem
            label={t("sessions.group.all")}
            count={totalCount}
            active={selectedGroupId === ALL_GROUPS}
            onClick={() => onSelect(ALL_GROUPS)}
          />
          {/* Only the custom group rows are sortable — the ALL / UNGROUPED
            sentinels stay outside the DragDropProvider so they can never move. */}
          <DragDropProvider sensors={sensors} onDragEnd={handleDragEnd}>
            {trackGroups.map((g, i) => (
              <GroupRow
                key={g.id}
                group={g}
                index={i}
                count={countById.get(g.id) ?? 0}
                active={selectedGroupId === g.id}
                onSelect={() => onSelect(g.id)}
                onRename={onRename}
                onDelete={onDelete}
                busy={busyGroupId === g.id}
              />
            ))}
          </DragDropProvider>
          {pendingGroup ? (
            <div className="text-muted-foreground flex items-center gap-2 rounded-md px-2 py-1.5 text-sm">
              <Loader2 className="size-3.5 animate-spin" />
              <span className="truncate">{pendingGroup}</span>
            </div>
          ) : null}
          <SidebarItem
            label={t("sessions.group.ungrouped")}
            count={ungroupedCount}
            active={selectedGroupId === UNGROUPED}
            onClick={() => onSelect(UNGROUPED)}
          />
        </div>
      </ScrollArea>
    </div>
  )
}

function SidebarItem({
  label,
  count,
  active,
  onClick,
}: {
  label: string
  count: number
  active: boolean
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "hover:bg-muted flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm transition-colors",
        active && "bg-muted text-foreground",
        !active && "text-muted-foreground",
      )}
    >
      <span className="flex-1 truncate">{label}</span>
      <span className="text-muted-foreground/70 text-xs tabular-nums">
        {count}
      </span>
    </button>
  )
}

function GroupRow({
  group: g,
  index,
  count,
  active,
  onSelect,
  onRename,
  onDelete,
  busy,
}: {
  group: SessionGroup
  index: number
  count: number
  active: boolean
  onSelect: () => void
  onRename: (g: SessionGroup, name: string) => Promise<void>
  onDelete: (g: SessionGroup) => Promise<void>
  busy: boolean
}) {
  const { t } = useTranslation()
  const [renaming, setRenaming] = useState(false)
  const [draft, setDraft] = useState(g.name)
  const [popoverOpen, setPopoverOpen] = useState(false)
  // Deleting goes through a confirmation dialog (unlike rename, the action is
  // destructive — sessions would silently move to Ungrouped otherwise). The
  // dialog closes before the mutation runs, matching how the popover acts.
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)
  // The whole row is the drag handle (no separate grip icon — the sidebar is
  // 176px wide); the sortable plugin applies the drag/shift transforms to the
  // ref'd element automatically. Busy rows are disabled: a rename/delete in
  // flight can't be reordered out from under.
  const { ref, isDragging } = useSortable({
    id: g.id,
    index,
    disabled: busy,
  })

  function startRename() {
    setDraft(g.name)
    setRenaming(true)
  }

  async function commitRename() {
    const name = draft.trim()
    setRenaming(false)
    setPopoverOpen(false)
    if (name && name !== g.name) {
      await onRename(g, name)
    }
  }

  function confirmDelete() {
    setPopoverOpen(false)
    setDeleteConfirmOpen(true)
  }

  async function executeDelete() {
    setDeleteConfirmOpen(false)
    await onDelete(g)
  }

  return (
    <div
      ref={ref}
      className={cn(
        "group/grow hover:bg-muted flex items-center gap-1 rounded-md px-2 py-1.5 text-sm transition-colors",
        active ? "bg-muted text-foreground" : "text-muted-foreground",
        busy && "opacity-60",
        // The dragged row floats above its siblings (the plugin fixes its
        // position + z-index) while the others make room via CSS transforms.
        isDragging && "opacity-60 shadow-sm",
      )}
    >
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-2 text-left"
        onClick={onSelect}
        disabled={busy}
      >
        {busy ? <Loader2 className="size-3.5 shrink-0 animate-spin" /> : null}
        <span className="flex-1 truncate">{g.name}</span>
      </button>
      {/* The count always sits flush right, matching the plain rows. The
        action slot next to it starts at zero width and stays clipped, so the
        ⋮ occupies no space at rest; on hover the slot expands and the count
        yields to it with a slide. Focus also expands the slot, keeping the
        trigger reachable by keyboard. */}
      <span className="text-muted-foreground/70 text-xs tabular-nums">
        {count}
      </span>
      <div className="w-0 overflow-hidden transition-[width] duration-150 ease-out group-hover/grow:w-6 group-focus-within/grow:w-6">
        <Popover open={popoverOpen} onOpenChange={setPopoverOpen}>
          <PopoverTrigger
            render={
              <Button
                variant="ghost"
                size="icon-xs"
                aria-label={t("common.edit")}
                disabled={busy}
              />
            }
          >
            <MoreHorizontal />
          </PopoverTrigger>
          <PopoverContent className="w-56 p-2" align="end">
            {renaming ? (
              <div className="flex items-center gap-1">
                <Input
                  value={draft}
                  onChange={(e) => setDraft(e.target.value)}
                  className="h-7"
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void commitRename()
                    if (e.key === "Escape") setRenaming(false)
                  }}
                />
                <Button variant="ghost" size="icon-sm" onClick={commitRename}>
                  <Check />
                </Button>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  onClick={() => setRenaming(false)}
                >
                  <X />
                </Button>
              </div>
            ) : (
              <div className="flex flex-col gap-0.5">
                <button
                  type="button"
                  className="hover:bg-muted flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm"
                  onClick={startRename}
                >
                  <Pencil className="size-3.5" />
                  {t("sessions.group.rename")}
                </button>
                <button
                  type="button"
                  className="text-destructive hover:bg-destructive/10 flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm"
                  onClick={confirmDelete}
                >
                  <Trash2 className="size-3.5" />
                  {t("sessions.group.delete")}
                </button>
              </div>
            )}
          </PopoverContent>
        </Popover>
      </div>
      {/* Confirmation before delete. Clicking the backdrop (or ESC) cancels
        and closes the dialog; only a deliberate click on 删除 runs the
        mutation. Sessions inside the group survive (they move to Ungrouped),
        so the description says so instead of threatening data loss. */}
      <ConfirmDialog
        open={deleteConfirmOpen}
        onOpenChange={setDeleteConfirmOpen}
        title={t("sessions.group.deleteConfirmTitle")}
        description={t("sessions.group.deleteConfirmDesc", { name: g.name })}
        confirmLabel={t("sessions.group.deleteConfirmAction")}
        busy={busy}
        onConfirm={() => void executeDelete()}
      />
    </div>
  )
}
