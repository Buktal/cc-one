// Token hero — token-first Tier 1 anchor. 总消耗大数字 + delta (多日: vs 窗首;
// 单日: vs 昨日同时段) + 日均/近 N 小时均，下接四桶堆叠 composition bar +
// 竖排 legend (label/value 行) + DSL footer (请求 · 命中率 · 成本).
//
// 概览区宽布局(#106 R1 定稿): 4/12 左卡竖排，与右侧 8/12 KPI 卡同高
// (justify-center 撑满)。颜色全部走 CSS 变量，换主题不改本件。
// delta/日均口径收敛在 use-token-snapshot（与 KPI 带共享同一次派生）。

import dayjs from "dayjs"
import { useTranslation } from "react-i18next"
import type { FilterState } from "@/app/store/slices/filterSlice"
import {
  BucketComposition,
  type CompositionSegment,
} from "@/components/bucket-composition"
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
import { BUCKET_DISPLAY, type BucketStatKey } from "@/lib/token-buckets"

export function TokenHero({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const {
    stats: s,
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
  // 展示名册 BUCKET_DISPLAY（lib/token-buckets）的 usage 域投影：构成条与
  // 图例共用同一序（序即名册序，与趋势图/会话统计同契约）；文案键与取数在
  // 本域拼接，呈现几何归 BucketComposition 原语。
  const segments: CompositionSegment[] = BUCKET_DISPLAY.map((b) => ({
    key: b.bucket,
    label: t(`usage.tokens.${b.suffix}`),
    color: b.cssVar,
    value: Number(s[`${b.bucket}_tokens` as BucketStatKey] ?? 0),
  }))

  return (
    <Card interactive className="h-full">
      <CardContent className="flex h-full flex-1 flex-col justify-center gap-3.5">
        {/* 总消耗大数字 + delta/日均 */}
        <div className="flex flex-col gap-2">
          <span className="text-muted-foreground text-xs">
            {t("usage.hero.total")}
          </span>
          <span className="text-[38px] leading-[1.05] font-semibold tracking-tight tabular-nums">
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

        {/* 四桶构成条 + 竖排 legend（原语单点）+ 口径行。值半行走 DSL
            `数量 · 占比`（占比恒一位小数）。 */}
        <BucketComposition
          segments={segments}
          total={total}
          renderValue={(value, share) =>
            formatSegValue(formatTokens(value), share)
          }
        />

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
      </CardContent>
    </Card>
  )
}
