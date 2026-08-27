// Shared confirmation dialog. Four delete surfaces (pricing rows, library
// entries, providers, session groups) show the same pattern: a title, an
// optional description, Cancel (an outline button), and a confirm button —
// destructive by default, carrying the same Trash2 icon as the row-level delete
// buttons so the danger identity lives on the confirm action alone and the
// header stays quiet. Non-destructive confirms (providers 缺必填切换确认) pass
// `destructive={false}`: quiet primary button, no icon — switching a provider
// is a heads-up, not a danger. Wraps the base-ui AlertDialog primitive —
// clicking the backdrop is treated as cancel (onBackdropClick →
// onOpenChange(false)); ESC already closes by default. Confirming still
// requires a deliberate click on the confirm button: focus lands on Cancel on
// open (initialFocus), so Enter only cancels; backdrop and ESC also only
// cancel.
//
// Two wiring styles, choose by whether the caller already has a per-row busy
// state:
// - Close-then-run (library, providers 切换确认): the row / toast shows its
//   own progress, so onConfirm closes the dialog and fires the action —
//   `busy` stays off.
// - Hold-open-until-done (pricing, providers delete): onConfirm awaits the
//   mutation and closes only on success — pass `busy` so the confirm button
//   shows a spinner and both buttons disable against double-clicks.
//
// Only the Cancel label is resolved here (the app-wide `common.cancel`) —
// everything else is plain props, like EmptyState, so callers keep their
// module's wording.

import { Loader2, Trash2 } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  description,
  confirmLabel,
  destructive = true,
  busy = false,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  confirmLabel: string
  /** 破坏性确认（删除类）：destructive 按钮 + Trash2 图标；false = quiet
   *  primary 按钮、无图标（如「缺必填仍切换」的提醒型确认）。 */
  destructive?: boolean
  /** Mutation in flight: both buttons disable, the confirm button spins. */
  busy?: boolean
  onConfirm: () => void
}) {
  const { t } = useTranslation()
  // Enter (and the initial focus) must land on Cancel, never on the
  // destructive action — deletion is always a deliberate click.
  const cancelRef = useRef<HTMLButtonElement>(null)
  // Callers clear their target (e.g. `deleting`) the moment the dialog closes,
  // so title/description would flash to their empty state during the fade-out
  // animation. Cache the last open content and render it while closing.
  const [lastOpen, setLastOpen] = useState({ title, description })
  useEffect(() => {
    if (open) setLastOpen({ title, description })
  }, [open, title, description])
  const shownTitle = open ? title : lastOpen.title
  const shownDescription = open ? description : lastOpen.description
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent
        onBackdropClick={() => onOpenChange(false)}
        initialFocus={cancelRef}
      >
        {/* 危险身份由确认按钮上的 Trash2 + destructive 样式承担，标题区保持
            安静——一个对话框只做一个强调点。 */}
        <AlertDialogHeader>
          <AlertDialogTitle>{shownTitle}</AlertDialogTitle>
          {shownDescription ? (
            <AlertDialogDescription>{shownDescription}</AlertDialogDescription>
          ) : null}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel ref={cancelRef} disabled={busy}>
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction
            variant={destructive ? "destructive" : "default"}
            disabled={busy}
            onClick={onConfirm}
          >
            {busy ? (
              <Loader2 className="animate-spin" />
            ) : destructive ? (
              <Trash2 />
            ) : null}
            {confirmLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
