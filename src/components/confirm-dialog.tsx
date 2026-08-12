// Shared destructive-action confirmation dialog. Four delete surfaces (pricing
// rows, library entries, providers, session groups) show the same pattern: a
// title, an optional description, Cancel, and a destructive confirm button.
// Wraps the base-ui AlertDialog primitive — clicking the backdrop is treated
// as cancel (onBackdropClick → onOpenChange(false)); ESC already closes by
// default, and the buttons always remain explicit. Deleting still requires a
// deliberate click on the destructive button — backdrop and ESC only cancel.
//
// Two wiring styles, choose by whether the caller already has a per-row busy
// state:
// - Close-then-delete (library, sessions): the row shows its own spinner, so
//   onConfirm closes the dialog and fires the mutation — `busy` stays off.
// - Hold-open-until-done (pricing, providers): onConfirm awaits the mutation
//   and closes only on success — pass `busy` so the confirm button shows a
//   spinner and both buttons disable against double-clicks.
//
// Only the Cancel label is resolved here (the app-wide `common.cancel`) —
// everything else is plain props, like EmptyState, so callers keep their
// module's wording.

import { Loader2 } from "lucide-react"
import { useEffect, useState } from "react"
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
  busy = false,
  onConfirm,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  description?: string
  confirmLabel: string
  /** Mutation in flight: both buttons disable, the confirm button spins. */
  busy?: boolean
  onConfirm: () => void
}) {
  const { t } = useTranslation()
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
      <AlertDialogContent onBackdropClick={() => onOpenChange(false)}>
        <AlertDialogHeader>
          <AlertDialogTitle>{shownTitle}</AlertDialogTitle>
          {shownDescription ? (
            <AlertDialogDescription>{shownDescription}</AlertDialogDescription>
          ) : null}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={busy}>
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={busy}
            onClick={onConfirm}
          >
            {busy ? <Loader2 className="animate-spin" /> : null}
            {confirmLabel}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
