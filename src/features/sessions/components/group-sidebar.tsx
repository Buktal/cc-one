// Group-row CRUD interactions for the sessions tree: the ⋮ actions popover
// (inline rename + delete-with-confirm) shared by every group node row —
// local groups rename/delete immediately; synced groups round-trip through
// git so their row shows a spinner while busy (the caller passes `busy`).
//
// Pure rendering — the CRUD handlers and the pending/busy flags come from
// useSessionsBrowser. The rename editor is the shared inline-edit pair
// (useInlineEdit owns draft/open/busy, InlineTextEdit owns the Enter / Escape /
// blur / ✓✕ contract); this component snapshots the group into the editor on
// begin and closes the popover when a commit lands.

import { Loader2, MoreHorizontal, Pencil, Trash2 } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { InlineTextEdit } from "@/components/inline-text-edit"
import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { useInlineEdit } from "@/hooks/use-inline-edit"
import type { SessionGroup } from "@/types/generated/bindings"

export function GroupActionsPopover({
  group: g,
  onRename,
  onDelete,
  busy,
}: {
  group: SessionGroup
  onRename: (g: SessionGroup, name: string) => Promise<void>
  onDelete: (g: SessionGroup) => Promise<void>
  /** A rename/delete in flight — the trigger disables and the row dims. */
  busy: boolean
}) {
  const { t } = useTranslation()
  const [popoverOpen, setPopoverOpen] = useState(false)
  // 行内重命名：draft/open/busy 机器归 useInlineEdit，键盘/失焦/按钮契约归
  // InlineTextEdit。target 存 begin 时抓的组对象——提交回调拿到的是改名前的
  // 组（快照语义，比对 name 不受列表重取影响）。onRename 以 Promise<void>
  // 上报（失败走 toast，resolve 不区分成败）：resolve = 完成（连弹层一起收
  // 起），reject = 失败留守编辑态；空名由 InlineTextEdit 挡下不可提交，
  // 未改名静默收尾（不打 mutation）。
  const rename = useInlineEdit<SessionGroup>({
    commit: async (target, draft) => {
      const name = draft.trim()
      if (name && name !== target.name) {
        try {
          await onRename(target, name)
        } catch {
          return false
        }
      }
      setPopoverOpen(false)
      return true
    },
  })
  const renaming = rename.target !== null
  // Deleting goes through a confirmation dialog (unlike rename, the action is
  // destructive — sessions would silently move to Ungrouped otherwise). The
  // dialog closes before the mutation runs, matching how the popover acts.
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)

  function confirmDelete() {
    setPopoverOpen(false)
    setDeleteConfirmOpen(true)
  }

  async function executeDelete() {
    setDeleteConfirmOpen(false)
    await onDelete(g)
  }

  return (
    <>
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
          {busy ? <Loader2 className="animate-spin" /> : <MoreHorizontal />}
        </PopoverTrigger>
        <PopoverContent className="w-56 p-2" align="end">
          {renaming ? (
            <InlineTextEdit
              value={rename.draft}
              onValueChange={rename.setDraft}
              busy={rename.busy}
              onCommit={() => void rename.commit()}
              onCancel={rename.cancel}
              ariaLabel={t("sessions.group.rename")}
              autoFocus
            />
          ) : (
            <div className="flex flex-col gap-0.5">
              <button
                type="button"
                className="hover:bg-hover flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm"
                onClick={() => rename.begin(g, g.name)}
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
      {/* Confirmation before delete. Clicking the backdrop (or ESC) cancels
          and closes the dialog; only a deliberate click on 删除 runs the
          mutation. Sessions inside the group survive (they move to
          Ungrouped), so the description says so instead of threatening data
          loss. */}
      <ConfirmDialog
        open={deleteConfirmOpen}
        onOpenChange={setDeleteConfirmOpen}
        title={t("sessions.group.deleteConfirmTitle")}
        description={t("sessions.group.deleteConfirmDesc", { name: g.name })}
        confirmLabel={t("sessions.group.deleteConfirmAction")}
        busy={busy}
        onConfirm={() => void executeDelete()}
      />
    </>
  )
}
