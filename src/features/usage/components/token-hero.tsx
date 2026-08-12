// Token hero — token-first Tier 1 anchor. 总消耗 headline + delta
// (vs 窗首) + daily avg + 四桶堆叠 composition bar + legend (label/value 行) +
// 缓存命中率 footer.
//
// 右栏窄布局: 纵向，无 sparkline — 中栏已有大趋势图，此处只留当前
// 窗口的数值快照。颜色全部走 CSS 变量，换主题不改本件。

import { useTranslation } from "react-i18next"
import { useStatsQuery, useTrendQuery, ZERO_STATS } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { Card, CardContent } from "@/components/ui/card"
import { tokenSnapshot } from "@/features/usage/derive"
import { formatInt, formatPct, formatTokens } from "@/lib/format"

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
  const { data: trend = [] } = useTrendQuery({ filter, bucket: "Day" })
  const s = stats ?? ZERO_STATS
  const total = s.total_tokens || 1
  // delta = 末日 vs 窗首 (trend 已按日升序); 日均 = 总量 / 窗口天数.
  const { deltaPct, dailyAvg } = tokenSnapshot(s, trend)

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
                  {t("usage.hero.vsStart")}
                </span>
              </span>
            ) : null}
            <span className="text-muted-foreground text-xs tabular-nums">
              {t("usage.hero.dailyAvg", { avg: formatTokens(dailyAvg) })}
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
            const pct = (v / total) * 100
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
                <span className="tabular-nums">
                  {formatTokens(v)} · {pct.toFixed(0)}%
                </span>
              </div>
            )
          })}
        </div>

        <div className="text-muted-foreground flex items-center justify-between border-border/60 border-t pt-2.5 text-xs">
          <span className="tabular-nums">
            {t("usage.hero.requests", { n: formatInt(s.request_count) })}
          </span>
          <span className="tabular-nums">
            {t("usage.hero.cacheHitRate", {
              rate: formatPct(s.cache_hit_rate),
            })}
          </span>
        </div>
      </CardContent>
    </Card>
  )
}
