import dayjs from "dayjs"
import { useTranslation } from "react-i18next"
import { Cell, Pie, PieChart } from "recharts"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
} from "@/components/ui/chart"
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
import { BUCKET_DISPLAY, type BucketStatKey } from "@/lib/token-buckets"

export function TokenHero({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const {
    stats: s,
    isLoading,
    error,
    retry,
    deltaPct,
    singleDay,
    dailyAvg,
    hourlyAvg,
  } = useTokenSnapshot(filter)
  const total = s.total_tokens
  const avgNode = singleDay
    ? t("usage.hero.hourlyAvg", {
        n: dayjs().hour() + 1,
        avg: formatTokens(hourlyAvg),
      })
    : t("usage.hero.dailyAvg", { avg: formatTokens(dailyAvg) })
  // 展示名册 BUCKET_DISPLAY（lib/token-buckets）的 usage 域投影：构成环与
  // 图例共用同一序（序即名册序，与趋势图/会话统计同契约）；文案键与取数在
  // 本域拼接。
  const segments = BUCKET_DISPLAY.map((b) => ({
    key: b.bucket,
    label: t(`usage.tokens.${b.suffix}`),
    color: b.cssVar,
    value: Number(s[`${b.bucket}_tokens` as BucketStatKey] ?? 0),
  }))
  // 环心读数：最大桶的占比（total=0 空窗 → null，环心显占位破折）。
  const lead = segments.reduce((m, seg) => (seg.value > m.value ? seg : m))
  const leadShare = total > 0 ? lead.value / total : null
  const chartConfig = Object.fromEntries(
    segments.map((seg): [string, ChartConfig[string]] => [
      seg.key,
      { label: seg.label, color: seg.color },
    ]),
  ) satisfies ChartConfig

  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.hero.total")}</CardTitle>
        <div className="token-ledger mt-1" aria-hidden="true">
          {segments.map((seg) => (
            <span key={seg.key} style={{ backgroundColor: seg.color }} />
          ))}
        </div>
      </CardHeader>
      <CardContent className="flex flex-1 flex-col justify-center gap-4">
        <QueryState
          isLoading={isLoading}
          error={error}
          isEmpty={false}
          errorAction={{ label: t("common.retry"), onClick: retry }}
        >
          {/* 总消耗大数字 + delta/日均 */}
          <div className="flex flex-col gap-2">
            <span className="token-total">{formatTokens(s.total_tokens)}</span>
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              {deltaPct !== null ? (
                <span className="inline-flex items-center gap-1 text-xs font-medium tabular-nums">
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

          {/* 四桶构成环 + 竖排 legend。环心 = 最大构成项占比（不重复顶部
            总量大字）。段间留缝靠 paddingAngle（卡面透出）；零值桶不进
            Pie。值半行走 DSL `数量 · 占比`（占比恒一位小数）。 */}
          <div className="flex flex-wrap items-center gap-4">
            <div className="relative shrink-0">
              <ChartContainer
                config={chartConfig}
                className="aspect-square size-32"
              >
                <PieChart>
                  <ChartTooltip
                    content={({ active, payload }) =>
                      active && payload?.length ? (
                        <div className="bg-popover rounded-md border p-2 text-xs shadow-sm">
                          <div className="flex items-center justify-between gap-4">
                            <span className="flex items-center gap-1">
                              <span
                                className="inline-block size-2 rounded-full"
                                style={{
                                  backgroundColor: segments.find(
                                    (seg) => seg.label === payload[0].name,
                                  )?.color,
                                }}
                              />
                              {payload[0].name}
                            </span>
                            <span className="tabular-nums">
                              {formatTokens(Number(payload[0].value ?? 0))}
                            </span>
                          </div>
                        </div>
                      ) : null
                    }
                  />
                  <Pie
                    data={segments.filter((seg) => seg.value > 0)}
                    dataKey="value"
                    nameKey="label"
                    innerRadius={44}
                    outerRadius={62}
                    paddingAngle={2}
                    strokeWidth={0}
                    isAnimationActive={false}
                  >
                    {segments
                      .filter((seg) => seg.value > 0)
                      .map((seg) => (
                        <Cell key={seg.key} fill={seg.color} />
                      ))}
                  </Pie>
                </PieChart>
              </ChartContainer>
              {/* 环心读数（绝对居中）：最大桶占比 + 桶名。 */}
              <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
                <span className="text-lg leading-none font-semibold tabular-nums">
                  {leadShare != null ? formatPct(leadShare) : "—"}
                </span>
                <span className="text-muted-foreground mt-1 max-w-20 truncate text-[10.5px]">
                  {leadShare != null ? lead.label : t("usage.hero.noUsage")}
                </span>
              </div>
            </div>
            <div className="flex min-w-48 flex-1 flex-col gap-1.5 text-xs">
              {segments.map((seg) => (
                <div key={seg.key} className="flex items-center gap-1.5">
                  <span
                    className="size-2 shrink-0 rounded-sm"
                    style={{ backgroundColor: seg.color }}
                  />
                  <span className="text-muted-foreground min-w-0 flex-1 truncate">
                    {seg.label}
                  </span>
                  <span className="shrink-0 tabular-nums">
                    {formatSegValue(
                      formatTokens(seg.value),
                      total > 0 ? seg.value / total : null,
                    )}
                  </span>
                </div>
              ))}
            </div>
          </div>

          {/* DSL footer 行：请求 · 命中率 · 成本（标签 数量 段拼装）。 */}
          <div className="text-muted-foreground border-border/60 mt-auto flex items-center border-t pt-2.5 text-xs">
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
        </QueryState>
      </CardContent>
    </Card>
  )
}
