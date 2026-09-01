// Usage trend chart — the four token buckets (input / output / cache
// creation / cache read) as a STACKED AREA group (#119 看板改版: 四条独立
// 折线升级为堆叠面积)。堆叠把「每桶各自多大」换成「四桶怎么构成、构成随
// 时间怎么迁移」——桶与桶的此消彼长在同卡内直接可读。一卡三模式：
//   abs   — 绝对值堆叠（原四折线的同一份数据，改为构成读法）；
//   share — 100% 占比堆叠（shareStackTrend）：每点按「可见桶总量」归一，
//           图例隐藏一桶 = 把它剔出构成（从分母剔除），而不是把它画成 0
//           ——隐藏读作「不参与构成」，剩余桶的占比始终归一；
//   cum   — 累计爬坡（cumulativeTrend，#119 二期）：窗口首点至今的 token
//           前缀和，单面积渐隐 + 底部端值行（累计读数不靠 hover）。斜率
//           即消耗加速度；单日窗口的逐小时累计同样成立。
// Colors flow straight from the semantic B-tier chart tokens (--chart-input
// / -output / -cache-create / -cache-read), so a skin swap changes the mood,
// never the meaning.
//
// NOTE: the spec's efficiency sub-charts (avg turn duration / request·turn by
// day) need per-day turn aggregates that TrendPoint does not carry today —
// only the global UsageStats has them. Daily turn trends require a backend
// change (extend TrendPoint); tracked in backlog.

import dayjs from "dayjs"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts"
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
import {
  cumulativeTrend,
  shareStackTrend,
  zeroFillTrend,
} from "@/features/usage/derive-trend"
import { tickIntervalFor } from "@/lib/chart"
import { dayRangeToTs, effectiveDays, sameDayWindow } from "@/lib/date-range"
import { formatCost, formatDay, formatPct, formatTokens } from "@/lib/format"
import { BUCKET_DISPLAY, type BucketStatKey } from "@/lib/token-buckets"
import { cn } from "@/lib/utils"

import type { TrendBucket, TrendPoint } from "@/types/generated/bindings"

type Bucket = {
  key: BucketStatKey
  name: string
  color: string
}

// 展示名册 BUCKET_DISPLAY（lib/token-buckets）的 usage 域投影。序即名册序
// （= KPI 带 / token hero / 会话统计条形的同一展示序契约），每条带跨皮肤
// 保持自己的色系；色/文案键不再手抄。
const BUCKETS: Bucket[] = BUCKET_DISPLAY.map((b) => ({
  key: `${b.bucket}_tokens`,
  name: `usage.tokens.${b.suffix}`,
  color: b.cssVar,
}))

/** 堆叠模式：abs = 绝对值堆叠；share = 100% 占比堆叠；cum = 累计爬坡。 */
type TrendMode = "abs" | "share" | "cum"

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
  // 堆叠模式 + 图例显隐集合。占比模式下「可见桶」集合是 shareStackTrend 的
  // 分母口径：隐藏的桶不参与构成。
  const [mode, setMode] = useState<TrendMode>("abs")
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
  const filled = useMemo(() => {
    if (!hourly || !fromTs || !toTs) return rawData
    return zeroFillTrend(rawData, dayjs(fromTs), dayjs(toTs), dayjs())
  }, [hourly, rawData, fromTs, toTs])

  // 占比模式：shareStackTrend 把每点四桶换成「占可见桶总量的比率」
  // （0–1），分母随图例显隐收窄；绝对模式照旧原样；累计模式换
  // cumulativeTrend 的前缀和点列（cum / dayTokens 两字段）。
  const data = useMemo(() => {
    if (mode === "cum") return cumulativeTrend(filled)
    if (mode !== "share") return filled
    const visible = new Set<BucketStatKey>(
      BUCKETS.filter((b) => !hidden.has(b.key)).map((b) => b.key),
    )
    return shareStackTrend(filled, visible)
  }, [mode, filled, hidden])

  // ChartConfig keys MUST equal the dataKeys (input_tokens …) so the shadcn
  // legend helper resolves label + color from payload.dataKey. stroke / fill
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
    <Card interactive className="h-full">
      <CardHeader>
        <CardTitle>{t("usage.trend.title")}</CardTitle>
        <CardAction>
          <div className="flex items-center gap-2">
            <span className="text-muted-foreground text-xs tabular-nums">
              {data.length > 0
                ? hourly
                  ? t("usage.trend.lastHours", { n: data.length })
                  : t("usage.trend.lastDays", { n: data.length })
                : t("usage.trend.noData")}
            </span>
            {/* 绝对值/占比/累计模式开关 —— 胶囊开关惯例逐字同 model-distribution
                的 header 写法（header 的 has-[card-action] 把开关待在自己的
                auto 宽列里右对齐，不被标题宽度拉伸）。 */}
            <div className="bg-muted/60 inline-flex items-center gap-0.5 rounded-md p-0.5">
              {(["abs", "share", "cum"] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => setMode(m)}
                  className={`rounded-[5px] px-2 py-0.5 text-xs font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/40 ${
                    mode === m
                      ? "bg-accent-tint text-accent-brand-strong shadow-sm"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  {m === "abs"
                    ? t("usage.trend.modeAbsolute")
                    : m === "share"
                      ? t("usage.trend.modeShare")
                      : t("usage.trend.modeCumulative")}
                </button>
              ))}
            </div>
          </div>
        </CardAction>
      </CardHeader>
      {/* flex-1：hero 同行配平时图区吃满卡身剩余高（卡随行拉伸）。 */}
      <CardContent className="flex flex-1 flex-col justify-center">
        <QueryState
          isLoading={isLoading}
          error={error}
          isEmpty={data.length === 0}
          emptyLabel={t("usage.trend.empty")}
          emptyDescription={t("usage.trend.emptyDesc")}
        >
          <ChartContainer config={chartConfig} className="h-72 w-full">
            <AreaChart
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
                fontSize={12}
                stroke="var(--muted-foreground)"
                // 占比模式固定 0–1（100% 堆叠满幅），绝对模式自适应。
                domain={mode === "share" ? [0, 1] : undefined}
                tickFormatter={(v) =>
                  mode === "share"
                    ? formatPct(Number(v))
                    : formatTokens(Number(v))
                }
              />
              <ChartTooltip
                content={<TrendTooltip hourly={hourly} mode={mode} />}
              />
              {mode === "cum" ? (
                /* 累计模式：总量单面积（四桶的「存量构成」堆叠会误导——堆叠
                   面积是每期构成，累计堆叠是存量读法，语义不同），渐隐填充 +
                   2px 主色描边；端值读数走图例位的累计行，不靠 hover。 */
                <Area
                  type="monotone"
                  dataKey="cum"
                  name={t("usage.trend.modeCumulative")}
                  stroke="var(--primary)"
                  strokeWidth={2}
                  fill="var(--primary)"
                  fillOpacity={0.12}
                  dot={false}
                  activeDot={{ r: 4, fill: "var(--primary)", strokeWidth: 0 }}
                  isAnimationActive={false}
                />
              ) : (
                /* stackId 共享 → 四带自下而上相接；带的顶缘描同色实线（1.5）
                    读得出每桶的边界；去逐点 dot（面积的语言是「带」不是「点」），
                    activeDot 只在 hover 处标位。 */
                BUCKETS.filter((b) => !hidden.has(b.key)).map((b) => (
                  <Area
                    key={b.key}
                    type="monotone"
                    dataKey={b.key}
                    name={t(b.name)}
                    stackId="tokens"
                    stroke={b.color}
                    strokeWidth={1.5}
                    fill={b.color}
                    fillOpacity={0.55}
                    dot={false}
                    activeDot={{ r: 4, fill: b.color, strokeWidth: 0 }}
                    isAnimationActive={false}
                  />
                ))
              )}
            </AreaChart>
          </ChartContainer>
          {mode === "cum" ? (
            /* 单系列无图例（标题即名）；图例位改放累计端值——爬坡终点的免
               hover 读数，与每日成本卡的顶标同职责。端值直接从源点列求和
               （data 是三态联合，不在此窄化）。 */
            <div className="flex items-center justify-center gap-1.5 pt-3 text-xs">
              <span
                className="size-2 shrink-0 rounded-[2px]"
                style={{ backgroundColor: "var(--primary)" }}
              />
              <span className="text-muted-foreground">
                {t("usage.trend.modeCumulative")}
              </span>
              <span className="font-medium tabular-nums">
                {formatTokens(
                  filled.reduce((s, p) => s + Number(p.total_tokens ?? 0), 0),
                )}
              </span>
            </div>
          ) : (
            <TrendLegend
              buckets={BUCKETS}
              hidden={hidden}
              onToggle={toggleLine}
            />
          )}
        </QueryState>
      </CardContent>
    </Card>
  )
}

/** 可点击图例 —— 点击切换单带显隐, 隐藏项半透明 + 划线。
 *  不用 recharts <Legend>: 它的 payload 只含「已渲染」的 series, 隐藏的带
 *  无法点回来; 手写图例始终渲染全部 4 项, 显隐只影响 Area 渲染。占比模式下
 *  隐藏一桶 = 把它从分母剔除（shareStackTrend 语义）, 剩余桶占比归一。 */
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
  /** 原始数据点 (recharts 注入)：占比模式从 `${dataKey}_abs` 取绝对量，
   *  桶外派生字段（total_tokens / total_cost_usd）也从这里取。 */
  payload?: { total_cost_usd?: number | null } & Record<string, unknown>
}

function TrendTooltip(props: {
  active?: boolean
  payload?: TooltipPayload[]
  label?: string
  hourly?: boolean
  mode?: TrendMode
}) {
  const { t } = useTranslation()
  const { active, payload, label, hourly, mode = "abs" } = props
  if (!active || !payload?.length) return null
  if (mode === "cum") {
    // 累计模式单系列：一行「累计 · 当日增量」两读。
    const point = payload[0]
    return (
      <div className="bg-popover rounded-md border p-2 text-xs shadow-sm">
        <div className="mb-1 font-medium">
          {label ? (hourly ? formatHour(label) : formatDay(label)) : ""}
        </div>
        <div className="flex items-center justify-between gap-4">
          <span>{t("usage.trend.cumTotal")}</span>
          <span className="tabular-nums">
            {formatTokens(Number(point.value ?? 0))}
          </span>
        </div>
        <div className="text-muted-foreground flex items-center justify-between gap-4">
          <span>{t("usage.trend.cumDayDelta")}</span>
          <span className="tabular-nums">
            +{formatTokens(Number(point.payload?.total_tokens ?? 0))}
          </span>
        </div>
      </div>
    )
  }
  const share = mode === "share"
  // 该时间点总成本 —— TrendPoint 自带, 补上「钱」的维度 (看板其他卡片
  // 没有成本的时间序列, 这里是最不打断浏览的位置)。
  const cost = payload[0]?.payload?.total_cost_usd ?? null
  // 合计行：占比模式显当日绝对总量（ShareTrendPoint.total_tokens，含隐藏
  // 桶——分母收窄只改占比口径，不抹掉当天体量）；绝对模式 = 可见桶之和。
  const total = share
    ? Number(payload[0]?.payload?.total_tokens ?? 0)
    : payload.reduce((sum, p) => sum + Number(p.value ?? 0), 0)
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
          <span className="tabular-nums">
            {share
              ? // 占比模式行值 =「占比 · 绝对量」，绝对量从 *_abs 旁路字段取。
                `${formatPct(Number(p.value))} · ${formatTokens(
                  Number(p.payload?.[`${p.dataKey}_abs`] ?? 0),
                )}`
              : formatTokens(Number(p.value))}
          </span>
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
