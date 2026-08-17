// Empty state — centered icon + title + optional description + optional
// action. Replaces the flat "暂无数据" string so empty surfaces can route the
// user to the next step (e.g. "采集本地日志" right from the table).

import type { LucideIcon } from "lucide-react"

import { Button } from "@/components/ui/button"

/** An action button on a state surface (empty / error) — label + handler. */
export type StateAction = {
  label: string
  onClick: () => void
  disabled?: boolean
}

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
}: {
  icon?: LucideIcon
  title: string
  description?: string
  action?: StateAction
}) {
  return (
    <div className="text-muted-foreground flex flex-col items-center justify-center gap-2 py-12 text-center">
      {Icon ? <Icon className="size-8 opacity-40" /> : null}
      <div className="text-foreground text-sm font-medium">{title}</div>
      {description ? <p className="max-w-sm text-xs">{description}</p> : null}
      {action ? (
        <Button
          size="sm"
          variant="outline"
          onClick={action.onClick}
          disabled={action.disabled}
          className="mt-1"
        >
          {action.label}
        </Button>
      ) : null}
    </div>
  )
}
