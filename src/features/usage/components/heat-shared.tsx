// 用量热力三形态（小时矩阵 / 月历 / 周历）的共享件：主题色阶、热力格、
// 整窗 Summary、少→多图例。色阶 = NONE + 四档主色渐进（25/50/75/100% 混进
// 卡面，color-mix）——热力档位有序，用主色深浅表达强度，与半环图的
// 「单色渐进阶」同一词汇；不借四桶色（那是构成类目，混进强度阶会串味），
// 中性 --heat-* 墨阶随之退役（index.css 已删）。

import { useTranslation } from "react-i18next"
import { formatCost, formatCount, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"

/** 热力格共有的读数形状（CalendarCell / HourMatrixCell 的结构子集）。 */
export interface HeatCellData {
  day: string
  tokens: number
  requests: number
  cost: number
  level: number
}

/** NONE（无用量）+ 四档主色渐进：level × 25% 的主色混进卡面。 */
export function heatColor(level: number): string {
  return level <= 0
    ? "var(--muted)"
    : `color-mix(in srgb, var(--primary) ${level * 25}%, var(--card))`
}

/** 热力格按钮：原生 title 承载逐格读数（365+ 格不逐格挂 React 浮层），
 *  点击 = 全局窗口收窄到该日（from = to = 该日，与 DistRow「点 = 收窄筛选」
 *  同一交互词汇）。showDayMark：大格内嵌日期号（深档翻卡面色保对比）。 */
export function HeatCell({
  cell,
  selected,
  onPickDay,
  showDayMark = false,
  className,
}: {
  cell: HeatCellData
  selected?: boolean
  onPickDay: (day: string) => void
  className?: string
}) {
  const { t } = useTranslation()
  return (
    <button
      type="button"
      // 原生 title：格内精确值靠它 + 点击收窄复看。
      title={[
        cell.day,
        formatTokens(cell.tokens),
        `${t("usage.hero.requests")} ${formatCount(cell.requests)}`,
        `${t("usage.caliber.priceEstimate")} ${formatCost(cell.cost)}`,
      ].join(" · ")}
      aria-label={t("usage.calendar.pickDay", { day: cell.day })}
      aria-pressed={selected}
      onClick={() => onPickDay(cell.day)}
      className={cn(
        "flex items-center justify-center overflow-visible rounded-[3px] outline-none focus-visible:ring-2 focus-visible:ring-ring/40",
        selected ? "ring-1 ring-ring" : "hover:ring-1 hover:ring-ring/50",
        className,
      )}
      style={{ backgroundColor: heatColor(cell.level) }}
    >
      {showDayMark ? (
        <span
          className={cn(
            "text-[10px] leading-none tabular-nums",
            cell.level >= 3 ? "text-card" : "text-muted-foreground",
          )}
        >
          {cell.day.slice(8)}
        </span>
      ) : null}
    </button>
  )
}

/** 共享的整窗聚合句（三形态同款「N contributions」读数，活跃按天计）。 */
export function HeatSummary({
  tokens,
  activeDays,
}: {
  tokens: number
  activeDays: number
}) {
  const { t } = useTranslation()
  return (
    <div className="text-muted-foreground pb-2 text-xs tabular-nums">
      {t("usage.calendar.summary", {
        tokens: formatTokens(tokens),
        days: activeDays,
      })}
    </div>
  )
}

/** 少→多五档图例（NONE + 四档主色）。 */
export function HeatLegend() {
  const { t } = useTranslation()
  return (
    <div className="text-muted-foreground flex items-center gap-1 text-[11px]">
      <span>{t("usage.calendar.less")}</span>
      {[0, 1, 2, 3, 4].map((l) => (
        <span
          key={l}
          className="size-[11px] shrink-0 rounded-[2px]"
          style={{ backgroundColor: heatColor(l) }}
        />
      ))}
      <span>{t("usage.calendar.more")}</span>
    </div>
  )
}
