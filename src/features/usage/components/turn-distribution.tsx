// 轮次分布卡（#119 四期重排：原 session-section 的半环卡拆出独立成卡）。
// 看板按「时间与分布 / 维度排行」两组相邻编排：半环分布属前者，会话排行
// 属后者（session-ranking），同源分区不再捆绑两卡。1–3 / 4–8 / 9–16 / 17+
// 四档半环（SemicircleChart），环心 = 窗口会话总数。数据 = 会话用量行
// （useSessionUsageQuery，与排行/KPI 带同一条查询缓存）→ sessionSectionStats
// 的 turnBuckets（桶划分会话集合，口径测试在 derive 侧）。

import { useTranslation } from "react-i18next"
import { useSessionUsageQuery } from "@/app/store/api"
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
import { sessionSectionStats } from "@/features/usage/derive"
import { formatCount } from "@/lib/format"

export function TurnDistribution({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const { data: rows = [], isLoading, error } = useSessionUsageQuery(filter)
  // topN = 0：本卡只读聚合（turnBuckets / sessions），Top 行在排行卡渲染。
  const stats = sessionSectionStats(rows, 0)
  // 四档展示名册（序 = 名册序、文案键尾段同源），取数用桶位。
  const tiers = [0, 1, 2, 3].map((i) => ({
    label: t(`usage.sessions.turnBand${i + 1}`),
    count: stats.turnBuckets[i],
  }))

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={rows.length === 0}
      emptyLabel={t("usage.sessions.empty")}
      emptyDescription={t("usage.sessions.emptyDesc")}
    >
      <Card interactive className="h-full">
        <CardHeader>
          <CardTitle>{t("usage.sessions.turnDistTitle")}</CardTitle>
          {/* 副标进 CardAction（全页 header 单行制，与时长分布卡同形）。 */}
          <CardAction>
            <span className="text-muted-foreground text-xs tabular-nums">
              {t("usage.sessions.bySessionCount")}
            </span>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-1 flex-col justify-center">
          <SemicircleChart
            tiers={tiers}
            centerValue={formatCount(stats.sessions)}
            centerLabel={t("usage.kpi.sessions")}
            formatValue={formatCount}
          />
        </CardContent>
      </Card>
    </QueryState>
  )
}
