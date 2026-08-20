// KPI 概览 — token-first Tier 2. 右栏列表式卡片：平均时长 /
// 请求·轮 / 总请求数 / 总成本。Token 总量在 TokenHero 锚点；缓存命中率在
// 锚点 footer。
//
// 右栏窄布局: 单列 label+value 行，替代旧 2×4 卡片网格。

import { useTranslation } from "react-i18next"
import { useStatsQuery, ZERO_STATS } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  formatCost,
  formatCount,
  formatDuration,
  formatRatio,
} from "@/lib/format"
import { cn } from "@/lib/utils"

export function KpiStrip({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const { data: stats } = useStatsQuery(filter)
  const s = stats ?? ZERO_STATS

  // 无轮次时无可算比率 — 占位破折号（与 formatDuration 的空值语义一致）。
  const perTurn =
    s.turn_count > 0 ? formatRatio(s.request_count / s.turn_count) : "—"

  // KPI 单值不带占比（DSL）；请求计数 ≥10K 才缩写，成本恒 $ 两位。
  const rows: Array<{ label: string; value: string; accent?: string }> = [
    {
      label: t("usage.kpi.avgDuration"),
      value: formatDuration(s.avg_turn_duration_ms),
    },
    { label: t("usage.kpi.perTurn"), value: perTurn },
    {
      label: t("usage.kpi.totalRequests"),
      value: formatCount(s.request_count),
    },
    {
      label: t("usage.kpi.totalCost"),
      value: formatCost(s.total_cost_usd),
      accent: "var(--metric-cost)",
    },
  ]

  return (
    <Card size="sm" interactive>
      <CardHeader>
        <CardTitle>{t("usage.kpi.title")}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col">
        {rows.map((r, i) => (
          <div
            key={r.label}
            className={cn(
              "flex items-baseline justify-between py-2",
              i > 0 && "border-border/60 border-t",
            )}
          >
            <span className="text-muted-foreground text-xs">{r.label}</span>
            <span
              className="text-lg font-semibold tabular-nums"
              style={r.accent ? { color: r.accent } : undefined}
            >
              {r.value}
            </span>
          </div>
        ))}
      </CardContent>
    </Card>
  )
}
