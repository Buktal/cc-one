// Daily cost chart — per-day total_cost_usd as a labeled bar column (#119:
// 条顶标数柱). 每日成本读数不靠 hover：柱顶直接标数（少类目「汇报图」口径，
// 原型 p1-bar-labeled 的生产落点）。14 根是顶标不互撞的边界——窗口更宽时只
// 显最近 14 桶（与请求分布条同一收缩规则），单日窗口退化为当日逐小时。柱一
// 色 = 一个度量（单序列），成本色走 --metric-cost（独立「钱」维度，非四桶）。
// 口径：total_cost_usd 是标价估算（与 ccusage calculate / Claude Code 本地
// 估算同类，与账单的差额是同类公开已知边界）——卡内常驻口径注。数据与趋势
// 图同一条 useTrendQuery 缓存（一 filterId 一份数据多图消费）。

import { useTranslation } from "react-i18next"
import { Bar, BarChart, LabelList, XAxis } from "recharts"
import { useTrendQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
} from "@/components/ui/chart"
import { tickIntervalFor } from "@/lib/chart"
import { dayRangeToTs, effectiveDays, sameDayWindow } from "@/lib/date-range"
import { formatCost, formatCount, formatDay } from "@/lib/format"
import type { TrendBucket } from "@/types/generated/bindings"

/** Bars shown — the prototype's "条顶标数柱" label-density ceiling (14 顶标
 *  不互撞; 单日窗口的小时桶 ≤ 24, 同样取最近 14). */
const BAR_COUNT = 14

const chartConfig = {
  total_cost_usd: { label: "Cost" },
} satisfies ChartConfig

export function DailyCostChart({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  // Bucket resolution mirrors the trend chart: a single local day collapses
  // per-day resolution to one bar, so zoom to hourly.
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  const hourly = sameDayWindow(fromTs, toTs)
  const bucket: TrendBucket = hourly ? "Hour" : "Day"
  const {
    data: trend = [],
    isLoading,
    error,
  } = useTrendQuery({ filter, bucket })
  const bars = trend.slice(-BAR_COUNT)
  // X 轴刻度间隔按桶数自适应（~7 个）—— 与趋势/请求分布条同规则。
  const tickInterval = tickIntervalFor(bars.length)

  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.costDaily.title")}</CardTitle>
        <CardAction>
          <span className="text-muted-foreground text-xs tabular-nums">
            {t("usage.caliber.priceEstimate")}
            {" · "}
            {hourly
              ? t("usage.costDaily.todayHours")
              : t("usage.costDaily.lastDays", { n: bars.length })}
          </span>
        </CardAction>
      </CardHeader>
      <CardContent>
        <QueryState
          isLoading={isLoading}
          error={error}
          isEmpty={bars.length === 0}
          emptyLabel={t("usage.costDaily.empty")}
          emptyDescription={t("usage.costDaily.emptyDesc")}
        >
          <ChartContainer config={chartConfig} className="h-44 w-full">
            <BarChart
              data={bars}
              margin={{ top: 18, right: 8, bottom: 0, left: 0 }}
            >
              <XAxis
                dataKey="day"
                interval={tickInterval}
                tickLine={false}
                axisLine={false}
                tickFormatter={(v) =>
                  hourly
                    ? `${String(v).slice(11, 13)}:00`
                    : formatDay(String(v))
                }
                fontSize={12}
                stroke="var(--muted-foreground)"
              />
              <ChartTooltip
                content={({ active, payload, label }) =>
                  active && payload?.length ? (
                    <div className="bg-popover rounded-md border p-2 text-xs shadow-sm">
                      <div className="mb-1 font-medium tabular-nums">
                        {hourly
                          ? `${String(label).slice(11, 13)}:00`
                          : formatDay(String(label))}
                      </div>
                      {/* 成本值 + 当桶请求量（TrendPoint 自带，标价口径已
                          常驻卡头，tooltip 不再重复）。 */}
                      <div className="tabular-nums">
                        {formatCost(Number(payload[0].value ?? 0))}
                        {" · "}
                        {t("usage.hero.requests")}{" "}
                        {formatCount(
                          Number(
                            (
                              payload[0].payload as
                                | { request_count?: number }
                                | undefined
                            )?.request_count ?? 0,
                          ),
                        )}
                      </div>
                    </div>
                  ) : null
                }
              />
              {/* maxBarSize 44：桶少时柱不铺满全宽，仍是一条柱而非一面墙。 */}
              <Bar
                dataKey="total_cost_usd"
                fill="var(--metric-cost)"
                fillOpacity={0.85}
                radius={[3, 3, 0, 0]}
                maxBarSize={44}
              >
                {/* 顶标 = 本形态的本体（免 hover 读数）。 */}
                <LabelList
                  dataKey="total_cost_usd"
                  position="top"
                  formatter={(v: unknown) => formatCost(Number(v ?? 0))}
                  fontSize={10}
                  fill="var(--muted-foreground)"
                />
              </Bar>
            </BarChart>
          </ChartContainer>
        </QueryState>
      </CardContent>
    </Card>
  )
}
