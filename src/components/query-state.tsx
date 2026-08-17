// Unified loading / error / empty state for RTK Query results.
// Empty renders <EmptyState> so callers can attach an icon, description and
// next-step action instead of a bare string.

import type { LucideIcon } from "lucide-react"
import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { EmptyState } from "@/components/empty-state"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { describeError } from "@/lib/error"

export function QueryState({
  isLoading,
  error,
  isEmpty,
  emptyLabel,
  emptyIcon,
  emptyDescription,
  emptyAction,
  errorAction,
  children,
}: {
  isLoading: boolean
  error: unknown
  isEmpty: boolean
  emptyLabel?: string
  emptyIcon?: LucideIcon
  emptyDescription?: string
  emptyAction?: { label: string; onClick: () => void; disabled?: boolean }
  /** Retry affordance on the error state (e.g. refetch) — a dead-end error
   *  with no way out is worse than the error itself. */
  errorAction?: { label: string; onClick: () => void; disabled?: boolean }
  children: ReactNode
}) {
  const { t } = useTranslation()
  if (isLoading) {
    return <Skeleton className="h-24 w-full rounded-md" />
  }
  if (error) {
    return (
      <div className="text-destructive flex flex-col items-start gap-2 text-sm">
        {t("common.loadFailed", {
          detail: describeError(error, t) || t("common.unknownError"),
        })}
        {errorAction ? (
          <Button variant="outline" size="sm" onClick={errorAction.onClick}>
            {errorAction.label}
          </Button>
        ) : null}
      </div>
    )
  }
  if (isEmpty) {
    return (
      // flex-1 centers the empty state in whatever space the list body would
      // have occupied — without it the empty block floats at the top while the
      // paged footer (or nothing) sits below, which reads as a broken layout.
      <div className="flex min-h-0 flex-1 items-center justify-center">
        <EmptyState
          icon={emptyIcon}
          title={emptyLabel ?? t("common.empty")}
          description={emptyDescription}
          action={emptyAction}
        />
      </div>
    )
  }
  return <>{children}</>
}
