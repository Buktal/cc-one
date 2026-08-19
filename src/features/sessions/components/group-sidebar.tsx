// Group-row CRUD interactions for the sessions tree: the ⋮ actions popover
// (inline rename + delete-with-confirm) shared by every group node row —
// local groups rename/delete immediately; synced groups round-trip through
// git so their row shows a spinner while busy (the caller passes `busy`).
//
// Pure rendering — the CRUD handlers and the pending/busy flags come from
// useSessionsBrowser. The rename input inside the popover is transient local
// state (the hook only learns the new name on submit).

import { Check, Loader2, MoreHorizontal, Pencil, Trash2, X } from "lucide-react"
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
  const [renaming, setRenaming] = useState(false)
  const [draft, setDraft] = useState(g.name)
  const [popoverOpen, setPopoverOpen] = useState(false)
  // Deleting goes through a confirmation dialog (unlike rename, the action is
  // destructive — sessions would silently move to Ungrouped otherwise). The
  // dialog closes before the mutation runs, matching how the popover acts.
  const [deleteConfirmOpen, setDeleteConfirmOpen] = useState(false)

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
                className="hover:bg-hover flex items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm"
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
