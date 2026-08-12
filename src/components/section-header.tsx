// Form section divider: a hairline with a small muted caption (e.g.
// 基本信息 / 价格 · 每百万 Token). Shared by provider-form-sheet and the
// pricing entry editor so grouped-field forms keep one visual language.
// `className` is for grid placement (e.g. `col-span-2`).

import { cn } from "@/lib/utils"

export function SectionHeader({
  className,
  children,
}: {
  className?: string
  children: React.ReactNode
}) {
  return (
    <div
      className={cn(
        "text-muted-foreground border-border/60 mt-3 border-t pt-2 text-[11px] font-semibold",
        className,
      )}
    >
      {children}
    </div>
  )
}
