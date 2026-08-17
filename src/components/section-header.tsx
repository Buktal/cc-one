// Form section divider: a muted caption (e.g. 基本信息 / 价格 · 每百万 Token)
// separated by spacing, not a hairline — the form's groups are told apart by
// air and type size, not by lines (provider-form-sheet 减线语言：线条留给控件
// 边框，分组靠间距 + 小标题)。字号与预设栏标题一致（text-sm），两处分区标题
// 同尺度。Shared by provider-form-sheet and the pricing entry editor so
// grouped-field forms keep one visual language.
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
        "text-muted-foreground mt-4 flex items-center justify-between gap-2 text-sm font-medium",
        className,
      )}
    >
      <span>{children}</span>
      {action ? <span className="shrink-0">{action}</span> : null}
    </div>
  )
}
