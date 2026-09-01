// 每日请求柱（#119 四期重排：原 request-section 的柱卡拆出独立成卡）。
// 时间与分布组的第二位（每日成本 > 每日请求相邻）：逐桶 request_count 柱，
// 峰值桶高亮（主色实心，其余 muted 半透——「哪天最忙」一眼可读，精确值靠
// tooltip）。桶分辨率与趋势/成本柱同规则：单日本地日窗口折叠成一根天桶 →
// 升小时粒度。近 14 桶（峰值标注可读的密度上限，单日窗口的小时桶 ≤ 24 同样
// 取最近 14）。数据与趋势图同一条 useTrendQuery 缓存（一 filterId 一份）。

import { useTranslation } from "react-i18next"
import { Bar, BarChart, Cell, XAxis } from "recharts"
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
import { requestHeadline, windowDayCount } from "@/features/usage/derive"
import { tickIntervalFor } from "@/lib/chart"
import { dayRangeToTs, effectiveDays, sameDayWindow } from "@/lib/date-range"
import { formatDay, formatInt } from "@/lib/format"
import type { TrendBucket } from "@/types/generated/bindings"

/** Bars shown — 峰值标注可读的密度上限（hour buckets on a single-day
 *  window shrink to the day's hours, which is ≤ 24). */
const BAR_COUNT = 14

const chartConfig = {
  request_count: { label: "Requests" },
} satisfies ChartConfig

export function DailyRequestChart({ filter }: { filter: FilterState }) {
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
  const headline = requestHeadline(bars, windowDayCount(from_day, to_day))
  const peakCount = headline.peakCount ?? 0
  // X 轴刻度间隔按桶数自适应（~7 个）—— 与趋势/成本柱同规则。
  const tickInterval = tickIntervalFor(bars.length)

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={bars.length === 0}
      emptyLabel={t("usage.requests.empty")}
      emptyDescription={t("usage.requests.emptyDesc")}
    >
      <Card interactive className="h-full">
        <CardHeader>
          {/* 主标题跟桶粒度：单日窗口实为当日逐小时，标题不再写死「每日」。 */}
          <CardTitle>
            {hourly
              ? t("usage.requests.todayTitle")
              : t("usage.requests.dailyTitle")}
          </CardTitle>
          {/* 副标进 CardAction（全页 header 单行制，与每日成本卡同形）。 */}
          <CardAction>
            <span className="text-muted-foreground text-xs tabular-nums">
              {hourly
                ? t("usage.requests.todayHours")
                : t("usage.requests.lastDays", { n: bars.length })}
            </span>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-1 flex-col justify-center">
          <ChartContainer config={chartConfig} className="h-44 w-full">
            <BarChart
              data={bars}
              margin={{ top: 8, right: 8, bottom: 0, left: 0 }}
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
                      <div className="tabular-nums">
                        {t("usage.requests.tooltipCount", {
                          n: formatInt(Number(payload[0].value ?? 0)),
                        })}
                        {Number(payload[0].value ?? 0) === peakCount
                          ? t("usage.requests.peakTag")
                          : null}
                      </div>
                    </div>
                  ) : null
                }
              />
              {/* maxBarSize 44：桶少时（单日 24 桶 / 单桶）柱不铺满全宽，
                  仍是一条柱而非一面墙。 */}
              <Bar
                dataKey="request_count"
                radius={[3, 3, 0, 0]}
                maxBarSize={44}
              >
                {bars.map((p) => (
                  <Cell
                    key={p.day}
                    fill={
                      p.request_count === peakCount && peakCount > 0
                        ? "var(--primary)"
                        : "var(--muted-foreground)"
                    }
                    fillOpacity={p.request_count === peakCount ? 1 : 0.55}
                  />
                ))}
              </Bar>
            </BarChart>
          </ChartContainer>
        </CardContent>
      </Card>
    </QueryState>
  )
}
