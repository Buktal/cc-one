// RelativeTime — 「相对时间 + 悬浮绝对时间」展示对的单一实现（架构审查
// Ⅲ候选②）。此对曾在 4 处各手写（会话列表 / 统计右栏身份卡 ×2 / 文件库
// 修改时间），悬浮侧还有 MM/DD 与 YYYY-MM-DD 两派——统一为 formatTimeExact：
// 相对措辞（「3 小时前」）已丢精度，悬浮里年份必须补全。
//
// fromNow 的语言随 dayjs 全局 locale（LanguageSync 驱动）；插件注册收口在
// @/i18n/languages，本组件不再自行 extend。空值渲染「—」（与 formatTime
// 的空值语义一致）。

import dayjs from "dayjs"

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatTimeExact } from "@/lib/format"
import { cn } from "@/lib/utils"

export function RelativeTime({
  ts,
  className,
  side,
}: {
  ts: string | number | null | undefined
  /** TooltipTrigger（即相对时间文本）的 className，如 tabular-nums。 */
  className?: string
  /** 悬浮层开口方向（右栏卡片传 left 不出列）；默认同 Tooltip 默认。 */
  side?: "top" | "bottom" | "left" | "right"
}) {
  if (!ts) return <span className={className}>—</span>
  return (
    <Tooltip>
      <TooltipTrigger render={<span className={cn(className)} />}>
        {dayjs(ts).fromNow()}
      </TooltipTrigger>
      <TooltipContent side={side}>{formatTimeExact(ts)}</TooltipContent>
    </Tooltip>
  )
}
