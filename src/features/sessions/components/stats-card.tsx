// 统计卡外壳（stats-cards 卡片组与 stats-identity-cards 身份卡共用）：小标题
// + 任意体。单独成件——两组卡片都要它，放进任何一组都会造出另一组对它的
// 值级反向 import（循环依赖）。

import type { ReactNode } from "react"
import { cn } from "@/lib/utils"

export function Card({
  title,
  children,
  className,
}: {
  title: string
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cn(
        "border-border bg-card/60 rounded-lg border p-2.5",
        className,
      )}
    >
      <h4 className="text-xs font-semibold">{title}</h4>
      <div className="mt-2">{children}</div>
    </section>
  )
}
