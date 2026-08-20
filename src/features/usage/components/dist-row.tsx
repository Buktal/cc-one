// DistRow — the dashboard sections' shared distribution row (#106): entity
// name + DSL value (`数量 · 占比`) on the first line, a share bar under it,
// and an optional sub line of secondary metrics. One row shape for the
// project ranking and the session/activity lists, so the sections read as one
// system. `hatch` marks aggregate rows (其他 / 未知项目) — the striped fill
// separates them from real entities (the #94/#96 decision).

import { cn } from "@/lib/utils"

export function DistRow({
  name,
  mono = false,
  value,
  share,
  sub,
  hatch = false,
  selected = false,
  onClick,
  ariaLabel,
}: {
  name: string
  /** Model/session ids render mono; localized names (未知项目) don't. */
  mono?: boolean
  /** Pre-formatted DSL value half (`数量 · 占比`) — the caller builds it with
   *  formatSegValue so the caliber stays in the DSL layer. */
  value: string
  /** [0,1] share driving the bar width. */
  share: number
  sub?: string
  hatch?: boolean
  selected?: boolean
  onClick?: () => void
  ariaLabel?: string
}) {
  const clickable = onClick != null
  return (
    <button
      type="button"
      disabled={!clickable}
      aria-pressed={clickable ? selected : undefined}
      aria-label={ariaLabel}
      onClick={onClick}
      className={cn(
        "group -mx-2 flex flex-col gap-1 rounded-md px-2 py-1.5 text-left",
        "disabled:cursor-default",
        selected ? "bg-accent-tint" : clickable && "hover:bg-hover",
      )}
    >
      <div className="flex items-baseline justify-between gap-2 text-xs">
        <span
          className={cn(
            "min-w-0 truncate font-medium",
            mono && "font-mono",
            selected && "text-accent-brand-strong",
          )}
        >
          {name}
        </span>
        <span className="text-muted-foreground shrink-0 tabular-nums">
          {value}
        </span>
      </div>
      <div className="bg-muted h-1.5 w-full overflow-hidden rounded-full">
        <div
          className={cn(
            "h-full rounded-full",
            hatch
              ? "bar-hatch"
              : selected
                ? "bg-primary"
                : "bg-primary/70 group-hover:bg-primary",
          )}
          style={{ width: `${Math.max(Math.min(share * 100, 100), 2)}%` }}
        />
      </div>
      {sub ? (
        <div className="text-muted-foreground/70 text-[10.5px] tabular-nums">
          {sub}
        </div>
      ) : null}
    </button>
  )
}
