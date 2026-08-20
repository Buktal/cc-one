// Token hero — token-first Tier 1 anchor. 总消耗 headline + delta
// (多日: vs 窗首; 单日: vs 昨日同时段) + 日均/近 N 小时均 + 四桶堆叠
// composition bar + legend (label/value 行) + DSL footer (请求 · 命中率 · 成本).
//
// 右栏窄布局: 纵向，无 sparkline — 中栏已有大趋势图，此处只留当前
// 窗口的数值快照。颜色全部走 CSS 变量，换主题不改本件。

import dayjs from "dayjs"
import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { useStatsQuery, useTrendQuery, ZERO_STATS } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { Card, CardContent } from "@/components/ui/card"
import { hourlySnapshot, tokenSnapshot } from "@/features/usage/derive"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import {
  formatCost,
  formatCount,
  formatMetricLine,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTokens,
} from "@/lib/format"
import type { TrendBucket } from "@/types/generated/bindings"

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
  const { data: stats } = useStatsQuery(filter)
  // 单日窗口 (today / 单日 custom) 判定与趋势图一致: 时间戳范围落在同一个
  // 本地日 → 小时粒度。此时多日的 delta/日均语义失真 (日趋势只有 1 个点,
  // delta 恒 null, 日均退化回总量), 改走 hourlySnapshot 的「vs 昨日同时段」。
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  const singleDay = !!fromTs && !!toTs && dayjs(fromTs).isSame(toTs, "day")
  // 昨日 filter —— 与当前 filter 同维度 (model/source/device), 只把窗口换成
  // 昨天的具体日期。queryKey 一天内稳定, 跨天滚动自动重查。
  const yesterdayFilter = useMemo<FilterState>(
    () =>
      singleDay
        ? {
            ...filter,
            range_preset: "custom",
            from_day: dayjs(from_day).subtract(1, "day").format("YYYY-MM-DD"),
            to_day: dayjs(from_day).subtract(1, "day").format("YYYY-MM-DD"),
          }
        : filter,
    [filter, singleDay, from_day],
  )
  const bucket: TrendBucket = singleDay ? "Hour" : "Day"
  const { data: trend = [] } = useTrendQuery({ filter, bucket })
  const { data: yesterday = [] } = useTrendQuery(
    { filter: yesterdayFilter, bucket: "Hour" },
    { skip: !singleDay },
  )
  const s = stats ?? ZERO_STATS
  const total = s.total_tokens || 1
  // 多日: delta = 末日 vs 窗首 (trend 已按日升序), 日均 = 总量 / 窗口天数;
  // 单日: delta = 今日 vs 昨日同时段, 均值 = 总量 / 已过小时数。
  const now = dayjs()
  const hourlySnap = singleDay ? hourlySnapshot(trend, yesterday, now) : null
  const multiSnap = singleDay ? null : tokenSnapshot(s, trend)
  const deltaPct = hourlySnap?.deltaPct ?? multiSnap?.deltaPct ?? null
  const avgNode = hourlySnap
    ? t("usage.hero.hourlyAvg", {
        n: now.hour() + 1,
        avg: formatTokens(hourlySnap.hourlyAvg),
      })
    : t("usage.hero.dailyAvg", { avg: formatTokens(multiSnap?.dailyAvg ?? 0) })

  return (
    <Card interactive>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
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

        <div className="flex flex-col gap-2">
          {SEGMENTS.map((seg) => {
            const v = Number(s[seg.key] ?? 0)
            return (
              <div
                key={seg.key}
                className="flex items-center justify-between text-xs"
              >
                <span className="flex items-center gap-1.5">
                  <span
                    className="inline-block size-2 rounded-sm"
                    style={{ backgroundColor: seg.color }}
                  />
                  <span className="text-muted-foreground">{t(seg.label)}</span>
                </span>
                {/* DSL: 数量 · 占比（占比恒一位小数，标签在行左由布局渲染）。 */}
                <span className="tabular-nums">
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
