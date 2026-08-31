// 持久结果横幅——操作完成 / 失败后要「驻留原地」的结果回执：用户读完、甚至
// 离开再回来仍需可见的面（导入报告、连接验证结果、校验错误）。瞬时反馈（已复
// 制、已保存这类一眼即逝的确认）走 toast，不用它。收编了 cc-switch / live-import
// / code-editor / settings 各处手抄的「成功 = emerald 块 / 失败 = destructive 块」
// 配方，视觉定档为单一形状：浅色底 + 细边框（border-x/40 + bg-x/5）+ 小图标，
// tone 只换语义色与默认图标；busy 用 dashed 边框示意「进行中、会翻篇」。
// 文案由调用方经 children 给（须已翻译），本组件不持任何 i18n 键。

import type { LucideIcon } from "lucide-react"
import { AlertCircle, CheckCircle2, Info, Loader2 } from "lucide-react"
import type { ReactNode } from "react"

import { cn } from "@/lib/utils"

export type InlineBannerTone = "success" | "error" | "info" | "busy"

/** tone → 语义色（边框 / 底 / 文字）与默认图标。busy 的图标自带旋转。 */
const TONES: Record<
  InlineBannerTone,
  { icon: LucideIcon; iconClass?: string; box: string }
> = {
  success: {
    icon: CheckCircle2,
    box: "border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400",
  },
  error: {
    icon: AlertCircle,
    box: "border-destructive/40 bg-destructive/5 text-destructive",
  },
  info: {
    icon: Info,
    box: "border-border bg-muted/50 text-muted-foreground",
  },
  busy: {
    icon: Loader2,
    iconClass: "animate-spin",
    box: "border-dashed bg-muted/50 text-muted-foreground",
  },
}

export function InlineBanner({
  tone,
  icon,
  className,
  children,
}: {
  /** 语义档：决定底色 / 边框 / 文字色与默认图标。 */
  tone: InlineBannerTone
  /** 覆盖该 tone 的默认图标；传 null 画纯文字横幅。 */
  icon?: LucideIcon | null
  /** 布局微调（如与上方内容的间距），配色配方仍由 tone 定。 */
  className?: string
  /** 横幅内容；多段内容纵向堆叠（自行用 block 元素分段）。 */
  children: ReactNode
}) {
  const preset = TONES[tone]
  const Icon = icon === null ? null : (icon ?? preset.icon)
  return (
    <div
      className={cn(
        "flex items-start gap-2 rounded-md border p-2 text-xs leading-relaxed",
        preset.box,
        className,
      )}
    >
      {Icon ? (
        <Icon className={cn("mt-0.5 size-3.5 shrink-0", preset.iconClass)} />
      ) : null}
      {/* min-w-0 让长报错文本（URL / 后端 detail）在弹窗窄幅下正常折行，
          不把横幅撑破。 */}
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  )
}
