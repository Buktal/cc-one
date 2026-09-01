// Request section (#106) — the request dimension: per-bucket request bars
// (day resolution; hour on a single-day window, the trend chart's own rule)
// with the peak bucket highlighted, and the turn-duration distribution
// (<10s / 10–30s / 30–60s / >60s) as a half-ring composition (#119 档位类
// 形态：进度条 → 半环，环心合计 + 右侧行保精确读数) with avg / P95. The
// bars read the SAME trend query the overview chart consumes (one cache
// entry per filter).

import { useTranslation } from "react-i18next"
import { Bar, BarChart, Cell, XAxis } from "recharts"
import { useStatsQuery, useTrendQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
} from "@/components/ui/chart"
import { SemicircleChart } from "@/features/usage/components/semicircle-chart"
import { requestHeadline, windowDayCount } from "@/features/usage/derive"
import { tickIntervalFor } from "@/lib/chart"
import { dayRangeToTs, effectiveDays, sameDayWindow } from "@/lib/date-range"
import { formatCount, formatDay, formatDuration, formatInt } from "@/lib/format"
import type { TrendBucket } from "@/types/generated/bindings"

/** Bars shown — the prototype's "近 14 天" window (hour buckets on a
 *  single-day window shrink to the day's hours, which is ≤ 24). */
const BAR_COUNT = 14

const chartConfig = {
  request_count: { label: "Requests" },
} satisfies ChartConfig

export function RequestSection({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const { data: stats } = useStatsQuery(filter)
  // Bucket resolution mirrors the trend chart: a single local day collapses
  // per-day resolution to one bar, so zoom to hourly.
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  // 单日本地日窗口折叠成一根天桶 → 升小时粒度；谓词归属 lib/date-range。
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
  // X 轴刻度间隔按桶数自适应（~7 个）—— 与趋势图同规则：preserveStartEnd
  // 在密桶时只保首尾两个刻度。
  const tickInterval = tickIntervalFor(bars.length)

  const durLabels = [
    t("usage.requests.durBand1"),
    t("usage.requests.durBand2"),
    t("usage.requests.durBand3"),
    t("usage.requests.durBand4"),
  ]
  const durBuckets = stats?.turn_duration_buckets ?? [0, 0, 0, 0]
  const durTotal = durBuckets.reduce((a, b) => a + b, 0)

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={bars.length === 0}
      emptyLabel={t("usage.requests.empty")}
      emptyDescription={t("usage.requests.emptyDesc")}
    >
      <div className="grid gap-3 min-[1080px]:grid-cols-12">
        <Card interactive className="min-[1080px]:col-span-7">
          <CardHeader>
            <CardTitle>{t("usage.requests.dailyTitle")}</CardTitle>
            <span className="text-muted-foreground self-end text-xs">
              {hourly
                ? t("usage.requests.todayHours")
                : t("usage.requests.lastDays", { n: bars.length })}
            </span>
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

        <Card interactive className="min-[1080px]:col-span-5">
          <CardHeader>
            <CardTitle>{t("usage.requests.durTitle")}</CardTitle>
            <span className="text-muted-foreground self-end text-xs">
              {t("usage.requests.durSub")}
            </span>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-center gap-2">
            <SemicircleChart
              tiers={durBuckets.map((count, i) => ({
                label: durLabels[i],
                count,
              }))}
              centerValue={formatCount(durTotal)}
              centerLabel={t("usage.requests.turns")}
              formatValue={formatCount}
            />
            <div className="border-border/60 mt-2 flex flex-col gap-1.5 border-t pt-2 text-xs">
              <DurRow label={t("usage.kpi.avgDuration")}>
                {formatDuration(stats?.avg_turn_duration_ms)}
              </DurRow>
              <DurRow label={t("usage.requests.p95")}>
                {formatDuration(stats?.p95_turn_duration_ms)}
              </DurRow>
            </div>
          </CardContent>
        </Card>
      </div>
    </QueryState>
  )
}

function DurRow({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-semibold tabular-nums">{children}</span>
    </div>
  )
}
