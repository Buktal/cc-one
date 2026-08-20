// Token hero — token-first Tier 1 anchor. 总消耗 headline + delta
// (多日: vs 窗首; 单日: vs 昨日同时段) + 日均/近 N 小时均 + 四桶堆叠
// composition bar + legend (label/value 行) + DSL footer (请求 · 命中率 · 成本).
//
// 概览区宽布局(#106): 与 KPI 带等高并排占概览首行左 1/3。颜色全部走 CSS
// 变量，换主题不改本件。delta/日均口径收敛在 use-token-snapshot（与 KPI 带
// 共享同一次派生）。

import dayjs from "dayjs"
import { useTranslation } from "react-i18next"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { Card, CardContent } from "@/components/ui/card"
import { useTokenSnapshot } from "@/features/usage/use-token-snapshot"
import {
  formatCost,
  formatCount,
  formatMetricLine,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTokens,
} from "@/lib/format"

const SEGMENTS = [
  {
    key: "input_tokens",
    label: "usage.tokens.input",
    color: "var(--chart-input)",
  },
  {
    key: "output_tokens",
    label: "usage.tokens.output",
    color: "var(--chart-output)",
  },
  {
    key: "cache_creation_tokens",
    label: "usage.tokens.cacheCreation",
    color: "var(--chart-cache-create)",
  },
  {
    key: "cache_read_tokens",
    label: "usage.tokens.cacheRead",
    color: "var(--chart-cache-read)",
  },
] as const

export function TokenHero({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const {
    stats: s,
    deltaPct,
    singleDay,
    dailyAvg,
    hourlyAvg,
  } = useTokenSnapshot(filter)
  const total = s.total_tokens || 1
  const avgNode = singleDay
    ? t("usage.hero.hourlyAvg", {
        n: dayjs().hour() + 1,
        avg: formatTokens(hourlyAvg),
      })
    : t("usage.hero.dailyAvg", { avg: formatTokens(dailyAvg) })

  return (
    <Card interactive className="h-full">
      <CardContent className="flex h-full flex-col gap-4">
        <div className="flex flex-1 flex-col justify-center gap-1.5">
          <span className="text-muted-foreground text-xs">
            {t("usage.hero.total")}
          </span>
          <span className="text-4xl font-semibold leading-none tabular-nums">
            {formatTokens(s.total_tokens)}
          </span>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {deltaPct !== null ? (
              <span
                className={`inline-flex items-center gap-1 text-xs font-medium tabular-nums ${
                  deltaPct >= 0 ? "text-primary" : "text-destructive"
                }`}
              >
                {deltaPct >= 0 ? "↑" : "↓"} {formatPct(Math.abs(deltaPct))}
                <span className="text-muted-foreground font-normal">
                  {singleDay
                    ? t("usage.hero.vsYesterday")
                    : t("usage.hero.vsStart")}
                </span>
              </span>
            ) : null}
            <span className="text-muted-foreground text-xs tabular-nums">
              {avgNode}
            </span>
          </div>
        </div>

        <div className="bg-muted flex h-2 w-full overflow-hidden rounded-full">
          {SEGMENTS.map((seg) => {
            const v = Number(s[seg.key] ?? 0)
            const pct = (v / total) * 100
            return (
              <div
                key={seg.key}
                className="h-full"
                style={{ width: `${pct}%`, backgroundColor: seg.color }}
              />
            )
          })}
        </div>

        <div className="grid grid-cols-2 gap-x-4 gap-y-2">
          {SEGMENTS.map((seg) => {
            const v = Number(s[seg.key] ?? 0)
            return (
              <div
                key={seg.key}
                className="flex items-center justify-between gap-1.5 text-xs"
              >
                <span className="flex min-w-0 items-center gap-1.5">
                  <span
                    className="inline-block size-2 shrink-0 rounded-sm"
                    style={{ backgroundColor: seg.color }}
                  />
                  <span className="text-muted-foreground truncate">
                    {t(seg.label)}
                  </span>
                </span>
                {/* DSL: 数量 · 占比（占比恒一位小数，标签在行左由布局渲染）。 */}
                <span className="shrink-0 tabular-nums">
                  {formatSegValue(formatTokens(v), v / total)}
                </span>
              </div>
            )
          })}
        </div>

        {/* DSL footer 行：请求 · 命中率 · 成本（标签 数量 段拼装）。 */}
        <div className="text-muted-foreground border-border/60 flex items-center border-t pt-2.5 text-xs">
          <span className="tabular-nums">
            {formatMetricLine([
              formatMetricSeg(
                t("usage.hero.requests"),
                formatCount(s.request_count),
              ),
              formatMetricSeg(
                t("usage.hero.cacheHitRate"),
                formatPct(s.cache_hit_rate),
              ),
              formatMetricSeg(
                t("usage.metric.cost"),
                formatCost(s.total_cost_usd),
              ),
            ])}
          </span>
        </div>
      </CardContent>
    </Card>
  )
}
