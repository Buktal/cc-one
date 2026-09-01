// Project section (#106) — the dashboard's project dimension at usage grain:
// a Top-5 ranking (click a row to narrow the shared project filter) with the
// unknown bucket rendered hatched like the "others" aggregate. The standalone
// data-quality card was retired (#119 二期): its unknown share / Top-3
// concentration numbers were fully covered by the hatched row here and the
// KPI band's project cell, so the card carried zero incremental information.
// All numbers flow the metric DSL; shares divide by the ranking's all-bucket
// total (项目维度内占比).

import { useTranslation } from "react-i18next"
import { useProjectUsageQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { type FilterState, patchFilter } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { projectRanking } from "@/features/usage/derive"
import {
  formatCost,
  formatCount,
  formatMetricLine,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTime,
  formatTokens,
} from "@/lib/format"
import { projectBasename } from "@/lib/paths"
import { DistRow } from "./dist-row"

const TOP_N = 5

export function ProjectSection({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const selected = useAppSelector((s) => s.filter.filter.project)
  const { data: rows = [], isLoading, error } = useProjectUsageQuery(filter)
  const ranking = projectRanking(rows, TOP_N)
  // 局部绑定让 JSX 闭包里的 narrowing 生效（ranking.unknown 属性访问在
  // 回调里不保持收窄）。
  const unknown = ranking.unknown
  const rest = ranking.rest

  const pick = (project: string) =>
    dispatch(patchFilter({ project: selected === project ? "" : project }))

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={rows.length === 0}
      emptyLabel={t("usage.projects.empty")}
      emptyDescription={t("usage.projects.emptyDesc")}
    >
      {/* 卡体由父级网格定位（#119 三期：项目排行并入概览网格 8/12），
          本分区只提供卡片本身。 */}
      <Card interactive className="h-full">
        <CardHeader>
          <CardTitle>{t("usage.projects.rankTitle")}</CardTitle>
          {/* 副标进 CardAction（全页 header 单行制）——第二行副标会把内容
              起点压低一行，与同行其它卡的 header 形制也不再齐。 */}
          <CardAction>
            <span className="text-muted-foreground text-xs tabular-nums">
              {t("usage.projects.topN", { n: TOP_N })}
            </span>
          </CardAction>
        </CardHeader>
        <CardContent className="flex flex-1 flex-col justify-center gap-1.5">
          {ranking.top.map((r) => (
            <DistRow
              key={r.project}
              mono
              name={projectBasename(r.project)}
              value={formatSegValue(
                formatTokens(r.total_tokens),
                r.total_tokens / ranking.totalTokens,
              )}
              share={r.total_tokens / ranking.totalTokens}
              sub={formatMetricLine([
                formatMetricSeg(
                  t("usage.kpi.sessions"),
                  formatCount(r.session_count),
                ),
                formatMetricSeg(
                  t("usage.hero.requests"),
                  formatCount(r.request_count),
                ),
                formatMetricSeg(
                  t("usage.metric.cost"),
                  formatCost(r.total_cost_usd),
                ),
                // 命中率（#119 维度行字段补全）——后端行自带；0 视为无
                // 缓存活动不渲染（与模型卡同一条渲染惯例）。
                ...(r.cache_hit_rate
                  ? [
                      formatMetricSeg(
                        t("usage.hero.cacheHitRate"),
                        formatPct(r.cache_hit_rate),
                      ),
                    ]
                  : []),
                formatMetricSeg(
                  t("usage.projects.lastActive"),
                  formatTime(r.last_active_at),
                ),
              ])}
              selected={selected === r.project}
              ariaLabel={projectBasename(r.project)}
              onClick={() => pick(r.project)}
            />
          ))}
          {unknown ? (
            <DistRow
              name={t("filter.unknownProject")}
              hatch
              value={formatSegValue(
                formatTokens(unknown.total_tokens),
                unknown.total_tokens / ranking.totalTokens,
              )}
              share={unknown.total_tokens / ranking.totalTokens}
              sub={t("usage.projects.unknownSub")}
              selected={selected === unknown.project}
              onClick={() => pick(unknown.project)}
            />
          ) : null}
          {rest ? (
            <DistRow
              name={t("usage.projects.others", { n: rest.count })}
              hatch
              value={formatSegValue(
                formatTokens(rest.tokens),
                rest.tokens / ranking.totalTokens,
              )}
              share={rest.tokens / ranking.totalTokens}
              sub={formatMetricLine([
                formatMetricSeg(
                  t("usage.kpi.sessions"),
                  formatCount(rest.sessions),
                ),
                formatMetricSeg(
                  t("usage.hero.requests"),
                  formatCount(rest.requests),
                ),
                formatMetricSeg(t("usage.metric.cost"), formatCost(rest.cost)),
              ])}
            />
          ) : null}
        </CardContent>
      </Card>
    </QueryState>
  )
}
