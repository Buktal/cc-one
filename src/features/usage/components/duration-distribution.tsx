// 时长分布卡（#119 四期重排：原 request-section 的半环卡拆出独立成卡）。
// 时间与分布组的尾卡：<10s / 10–30s / 30–60s / >60s 轮次时长四档半环
// （SemicircleChart，环心 = 轮次总数），下接 avg / P95 精确读数行。数据 =
// useStatsQuery 的窗口聚合（turn_duration_buckets 由后端按同口径分桶，与
// KPI 带同一条查询缓存）。

import { useTranslation } from "react-i18next"
import { useStatsQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { SemicircleChart } from "@/features/usage/components/semicircle-chart"
import { formatCount, formatDuration } from "@/lib/format"

export function DurationDistribution({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const { data: stats, isLoading, error } = useStatsQuery(filter)
  const durBuckets = stats?.turn_duration_buckets ?? [0, 0, 0, 0]
  const durTotal = durBuckets.reduce((a, b) => a + b, 0)
  // 四档展示名册（序 = 名册序、文案键尾段同源），取数用桶位。
  const tiers = [0, 1, 2, 3].map((i) => ({
    label: t(`usage.requests.durBand${i + 1}`),
    count: durBuckets[i],
  }))

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      // 桶全零 = 窗口没有可分档的轮次（如纯空窗）→ 空态而非空环。
      isEmpty={durTotal === 0}
      emptyLabel={t("usage.requests.empty")}
      emptyDescription={t("usage.requests.emptyDesc")}
    >
      <Card interactive className="h-full">
        <CardHeader>
          <CardTitle>{t("usage.requests.durTitle")}</CardTitle>
          {/* 副标进 CardAction（全页 header 单行制，与轮次分布卡同形）。 */}
          <CardAction>
            <span className="text-muted-foreground text-xs tabular-nums">
              {t("usage.requests.durSub")}
            </span>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-1 flex-col justify-center gap-2">
          <SemicircleChart
            tiers={tiers}
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
