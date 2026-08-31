// BucketComposition — the token four-bucket composition primitive (架构审查Ⅶ
// C3 收尾)：堆叠构成条（token-hero 的 h-2.5 rounded-full 基础形）+ 竖排图例，
// usage hero 与会话统计右栏这对孪生的唯一实现。数据侧名册 BUCKET_DISPLAY
// （lib/token-buckets）早已单源，这里补上呈现侧：调用方把名册投影成 segments
// 喂入——文案键在本域解析（usage.tokens.* vs sessions.stats.bucket.*），值
// 半行与条上悬浮各自注入，DSL 口径不进原语；原语只持几何、名册序与占位段
// 钳制（shareBarPct：值 >0 的段至少 2% 可见，0 值桶不渲染——幻影段会虚报
// 不存在的占比）。

import type { ReactNode } from "react"
import { shareBarPct } from "@/lib/format"
import { cn } from "@/lib/utils"

/** 名册投影行：调用方把 BUCKET_DISPLAY × 本域数据 × 本域文案拼成行喂入。 */
export interface CompositionSegment {
  /** React key — the bucket key. */
  key: string
  /** Already-translated label — i18n stays in the caller's domain. */
  label: string
  /** Display color — the roster's `var(--chart-*)` reference. */
  color: string
  value: number
}

export function BucketComposition({
  segments,
  total,
  renderValue,
  segmentTitle,
  compact = false,
}: {
  segments: readonly CompositionSegment[]
  /** 占比分母 + 空态闸：≤0 渲染空条且 share 全为 null（值半行按调用域的
   *  空档口径呈现）。 */
  total: number
  /** 占比呈现策略 — 图例值半行由调用域拼装（usage hero：`数量 · 占比` 一
   *  串；会话右栏：值列 + 独立占比列）。share null = 总量为 0。 */
  renderValue: (value: number, share: number | null) => ReactNode
  /** 条段的悬浮文案（会话右栏的 `标签 数量 · 占比`）；不传不挂 tooltip
   *  （hero 的条贴着图例，无需悬浮）。 */
  segmentTitle?: (seg: CompositionSegment, share: number | null) => string
  /** 图例字号档：默认 hero 的 xs；会话右栏窄卡用 compact（[11px]、行距更紧）。 */
  compact?: boolean
}) {
  const shareOf = (value: number): number | null =>
    total > 0 ? value / total : null
  return (
    <div className="flex flex-col gap-2">
      <div className="bg-muted flex h-2.5 w-full overflow-hidden rounded-full">
        {total > 0
          ? segments
              .filter((seg) => seg.value > 0)
              .map((seg) => (
                <div
                  key={seg.key}
                  className="h-full"
                  style={{
                    width: `${shareBarPct(seg.value / total)}%`,
                    backgroundColor: seg.color,
                  }}
                  title={segmentTitle?.(seg, shareOf(seg.value))}
                />
              ))
          : null}
      </div>
      <div
        className={cn(
          "flex flex-col",
          compact ? "gap-1 text-[11px]" : "gap-1.5 text-xs",
        )}
      >
        {segments.map((seg) => (
          <div key={seg.key} className="flex items-center gap-1.5">
            <span
              className="size-2 shrink-0 rounded-sm"
              style={{ backgroundColor: seg.color }}
            />
            <span className="text-muted-foreground min-w-0 flex-1 truncate">
              {seg.label}
            </span>
            {/* 值半行容器：tabular-nums 与右缘锚定归原语，内容归调用域。 */}
            <span className="flex shrink-0 items-center gap-1.5 tabular-nums">
              {renderValue(seg.value, shareOf(seg.value))}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}
