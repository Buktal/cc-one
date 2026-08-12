// Form section divider: a hairline with a small muted caption (e.g.
// 基本信息 / 价格 · 每百万 Token). Shared by provider-form-sheet and the
// pricing entry editor so grouped-field forms keep one visual language.
// `className` is for grid placement (e.g. `col-span-2`). `action` is an
// optional right-aligned slot (e.g. the model-mapping fetch buttons); when
// absent the header renders exactly as before.

import { cn } from "@/lib/utils"

export function SectionHeader({
  className,
  action,
  children,
}: {
  className?: string
  /** 右侧操作区（按钮组）；不传则标题独占一行。 */
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <div
      className={cn(
        "text-muted-foreground border-border/60 mt-3 flex items-center justify-between gap-2 border-t pt-2 text-[11px] font-semibold",
        className,
      )}
    >
      <span>{children}</span>
      {action ? <span className="shrink-0">{action}</span> : null}
    </div>
  )
}
