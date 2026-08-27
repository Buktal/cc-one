// Usage trend chart: multi-line chart of the four token
// buckets (input / output / cache creation / cache read). Each bucket is its
// own INDEPENDENT line with a solid dot per data point — no stacked area, no
// fill under the line. The point is to compare each bucket's trend over time,
// not to read cumulative composition.
//
// Built on the shadcn Chart primitive (ChartContainer / ChartConfig /
// ChartLegend — see src/components/ui/chart.tsx). Colors flow straight from
// the semantic B-tier chart tokens (--chart-input / -output / -cache-create /
// -cache-read), so a skin swap changes the mood, never the meaning.
//
// NOTE: the spec's efficiency sub-charts (avg turn duration / request·turn by
// day) need per-day turn aggregates that TrendPoint does not carry today —
// only the global UsageStats has them. Daily turn trends require a backend
// change (extend TrendPoint); tracked in backlog.

import dayjs from "dayjs"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts"
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
import { zeroFillTrend } from "@/features/usage/derive"
import { tickIntervalFor } from "@/lib/chart"
import { dayRangeToTs, effectiveDays, sameDayWindow } from "@/lib/date-range"
import { formatCost, formatDay, formatTokens } from "@/lib/format"
import { BUCKET_DISPLAY, type BucketStatKey } from "@/lib/token-buckets"
import { cn } from "@/lib/utils"

import type { TrendBucket, TrendPoint } from "@/types/generated/bindings"

type Bucket = {
  key: BucketStatKey
  name: string
  color: string
}

// 展示名册 BUCKET_DISPLAY（lib/token-buckets）的 usage 域投影。序即名册序
// （= KPI 带 / token hero / 会话统计条形的同一展示序契约），每条线跨皮肤
// 保持自己的色系；色/文案键不再手抄。
const BUCKETS: Bucket[] = BUCKET_DISPLAY.map((b) => ({
  key: `${b.bucket}_tokens`,
  name: `usage.tokens.${b.suffix}`,
  color: b.cssVar,
}))

/** Hour bucket key `YYYY-MM-DDTHH` → `HH:00` for the axis / tooltip. */
function formatHour(key: string): string {
  return `${key.slice(11, 13)}:00`
}

export function UsageTrendChart({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  // Derive the timestamp bounds at render time: a dynamic preset
  // re-rolls to the current day here, so the hourly check + zero-fill track
  // the live window with no frozen value.
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  // A single local-day range collapses per-day resolution to one bar, so zoom
  // to hourly; anything wider stays per-day. 谓词归 lib/date-range（时区语义见其注释）。
  const hourly = sameDayWindow(fromTs, toTs)
  const bucket: TrendBucket = hourly ? "Hour" : "Day"
  // 图例显隐集合 —— 点击图例 toggle 单线显隐。隐藏的桶不再渲染 Line
  // (tooltip 也不含), 但手写图例始终全量, 保证能点回来。
  const [hidden, setHidden] = useState<ReadonlySet<string>>(() => new Set())
  const toggleLine = (key: string) =>
    setHidden((prev) => {
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  const {
    data: rawData = [],
    isLoading,
    error,
  } = useTrendQuery({
    filter,
    bucket,
  })

  // Hourly zero-fill: pin the x-axis to the selected local day instead of the
  // first→last bucket that has records. The backend GROUP BY only emits
  // buckets with rows, so without padding the 00:00→first-record stretch is
  // swallowed and the range looks truncated. For the current day the axis
  // stops at the current hour; a past single day fills the full 24h. Day
  // buckets (7d/30d/all) are left as-is; an entirely empty day stays empty so
  // QueryState shows its empty state rather than a flat zero line.
  const data = useMemo(() => {
    if (!hourly || !fromTs) return rawData
    return zeroFillTrend(rawData, dayjs(fromTs), dayjs())
  }, [hourly, rawData, fromTs])

  // ChartConfig keys MUST equal the dataKeys (input_tokens …) so the shadcn
  // legend helper resolves label + color from payload.dataKey. stroke / dot
  // use the bucket's own color directly (var(--chart-*)), not the
  // ChartStyle-injected --color-<key>. 整表由展示名册 reduce 生成——色/文案键
  // 唯一出处是 lib/token-buckets 的 BUCKET_DISPLAY，这里不再重抄。
  // X 轴刻度间隔按点数自适应（~7 个刻度）：preserveStartEnd 在 24 小时点上
  // 只保首尾两个、中间刻度全被丢，长窗口与短窗口都看不清节奏。
  const tickInterval = tickIntervalFor(data.length)
  const chartConfig: ChartConfig = Object.fromEntries(
    BUCKET_DISPLAY.map((b): [string, ChartConfig[string]] => [
      `${b.bucket}_tokens`,
      { label: t(`usage.tokens.${b.suffix}`), color: b.cssVar },
    ]),
  )

  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.trend.title")}</CardTitle>
        <CardAction>
          <span className="text-muted-foreground text-xs tabular-nums">
            {data.length > 0
              ? hourly
                ? t("usage.trend.lastHours", { n: data.length })
                : t("usage.trend.lastDays", { n: data.length })
              : t("usage.trend.noData")}
          </span>
        </CardAction>
      </CardHeader>
      <CardContent>
        <QueryState
          isLoading={isLoading}
          error={error}
          isEmpty={data.length === 0}
          emptyLabel={t("usage.trend.empty")}
          emptyDescription={t("usage.trend.emptyDesc")}
        >
          <ChartContainer config={chartConfig} className="h-72 w-full">
            <LineChart
              data={data}
              margin={{ top: 8, right: 24, bottom: 0, left: 0 }}
            >
              <CartesianGrid
                vertical={false}
                strokeDasharray="3 3"
                stroke="var(--border)"
              />
              <XAxis
                dataKey="day"
                interval={tickInterval}
                tickLine={false}
                axisLine={false}
                tickFormatter={(v) =>
                  hourly ? formatHour(String(v)) : formatDay(String(v))
                }
                fontSize={12}
                stroke="var(--muted-foreground)"
              />
              <YAxis
                tickLine={false}
                axisLine={false}
                tickFormatter={(v) => formatTokens(Number(v))}
                fontSize={12}
                stroke="var(--muted-foreground)"
              />
              <ChartTooltip content={<TrendTooltip hourly={hourly} />} />
              {BUCKETS.filter((b) => !hidden.has(b.key)).map((b) => (
                <Line
                  key={b.key}
                  type="monotone"
                  dataKey={b.key}
                  name={t(b.name)}
                  stroke={b.color}
                  strokeWidth={2}
                  dot={{ r: 3, fill: b.color, strokeWidth: 0 }}
                  activeDot={{ r: 5, fill: b.color, strokeWidth: 0 }}
                  isAnimationActive={false}
                />
              ))}
            </LineChart>
          </ChartContainer>
          <TrendLegend
            buckets={BUCKETS}
            hidden={hidden}
            onToggle={toggleLine}
          />
        </QueryState>
      </CardContent>
    </Card>
  )
}

/** 可点击图例 —— 点击切换单线显隐, 隐藏项半透明 + 划线。
 *  不用 recharts <Legend>: 它的 payload 只含「已渲染」的 series, 隐藏的线
 *  无法点回来; 手写图例始终渲染全部 4 项, 显隐只影响 Line 渲染。 */
function TrendLegend({
  buckets,
  hidden,
  onToggle,
}: {
  buckets: Bucket[]
  hidden: ReadonlySet<string>
  onToggle: (key: string) => void
}) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center justify-center gap-4 pt-3">
      {buckets.map((b) => {
        const off = hidden.has(b.key)
        return (
          <button
            key={b.key}
            type="button"
            aria-pressed={!off}
            aria-label={t(b.name)}
            onClick={() => onToggle(b.key)}
            className={cn(
              "flex items-center gap-1.5 rounded-sm outline-none transition-opacity focus-visible:ring-2 focus-visible:ring-ring/40",
              off && "opacity-40",
            )}
          >
            <span
              className={cn(
                "size-2 shrink-0 rounded-[2px]",
                off && "opacity-60",
              )}
              style={{ backgroundColor: b.color }}
            />
            <span
              className={cn(
                "text-xs",
                off && "text-muted-foreground line-through",
              )}
            >
              {t(b.name)}
            </span>
          </button>
        )
      })}
    </div>
  )
}

type TooltipPayload = {
  dataKey: string
  value: number | null
  name: string
  color: string
  /** 原始数据点 (recharts 注入), 用于取桶外的派生字段。 */
  payload?: { total_cost_usd?: number | null }
}

function TrendTooltip(props: {
  active?: boolean
  payload?: TooltipPayload[]
  label?: string
  hourly?: boolean
}) {
  const { t } = useTranslation()
  const { active, payload, label, hourly } = props
  if (!active || !payload?.length) return null
  const total = payload.reduce((sum, p) => sum + Number(p.value ?? 0), 0)
  // 该时间点总成本 —— TrendPoint 自带, 补上「钱」的维度 (看板其他卡片
  // 没有成本的时间序列, 这里是最不打断浏览的位置)。
  const cost = payload[0]?.payload?.total_cost_usd ?? null
  return (
    <div className="bg-popover rounded-md border p-2 text-xs shadow-sm">
      <div className="mb-1 font-medium">
        {label ? (hourly ? formatHour(label) : formatDay(label)) : ""}
      </div>
      {payload.map((p) => (
        <div
          key={p.dataKey}
          className="flex items-center justify-between gap-4"
        >
          <span className="flex items-center gap-1">
            <span
              className="inline-block size-2 rounded-full"
              style={{ backgroundColor: p.color }}
            />
            {p.name}
          </span>
          <span className="tabular-nums">{formatTokens(Number(p.value))}</span>
        </div>
      ))}
      <div className="mt-1 flex items-center justify-between gap-4 border-t pt-1 font-medium">
        <span>{t("usage.trend.total")}</span>
        <span className="tabular-nums">{formatTokens(total)}</span>
      </div>
      {cost != null ? (
        <div className="flex items-center justify-between gap-4 text-muted-foreground">
          <span>{t("usage.kpi.totalCost")}</span>
          <span className="tabular-nums">{formatCost(Number(cost))}</span>
        </div>
      ) : null}
    </div>
  )
}

export type { TrendPoint }
