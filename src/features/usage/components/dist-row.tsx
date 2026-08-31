// DistRow — the dashboard sections' shared distribution row (#106): entity
// name + DSL value (`数量 · 占比`) on the first line, a share bar under it,
// and an optional sub line of secondary metrics. One row shape for the
// project ranking, the session/activity lists and the device list, so the
// sections read as one system. `hatch` marks aggregate rows (其他 / 未知项目)
// — the striped fill separates them from real entities (the #94/#96
// decision). `badge` renders a tiny chip beside the name (the device
// section's「本机」mark).

import { Badge } from "@/components/ui/badge"
import { shareBarPct } from "@/lib/format"
import { cn } from "@/lib/utils"

export function DistRow({
  name,
  mono = false,
  badge,
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
  /** Tiny chip after the name — e.g. the「本机」mark on this machine's row. */
  badge?: string
  /** Pre-formatted DSL value half (`数量 · 占比`) — the caller builds it with
   *  formatSegValue so the caliber stays in the DSL layer. */
  value: string
  /** [0,1] share driving the bar width — shareBarPct clamps it (sliver floor
   *  + overflow ceiling), the row system's single clamp. */
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
        <span className="flex min-w-0 items-baseline gap-1">
          <span
            className={cn(
              "min-w-0 truncate font-medium",
              mono && "font-mono",
              selected && "text-accent-brand-strong",
            )}
          >
            {name}
          </span>
          {badge ? (
            <Badge
              variant="outline"
              className="text-muted-foreground h-4 shrink-0 px-1 text-[9.5px] font-medium"
            >
              {badge}
            </Badge>
          ) : null}
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
          style={{ width: `${shareBarPct(share)}%` }}
        />
      </div>
      {sub ? (
        <div className="text-muted-foreground text-[11px] tabular-nums">
          {sub}
        </div>
      ) : null}
    </button>
  )
}
