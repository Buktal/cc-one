// 表单字段原子：Label（muted xs）+ 控件竖排。Shared by the providers form
// sheet and the pricing entry editor（判例：SectionHeader——同为双表单共享的
// 表单原子住 src/components）；pricing 曾在 entry-editor-dialog 本地抄一份
// （无 className 参数），共享份住 feature 层导致它只能抄（架构审查Ⅲ候选⑫
// 收敛）。`className` 用于布局（flex 子项的伸缩 / 宽度约束）。

import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"

export function Field({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <Label className="text-muted-foreground text-xs">{label}</Label>
      {children}
    </div>
  )
}
