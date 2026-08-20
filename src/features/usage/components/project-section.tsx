// Project section (#106) — the dashboard's project dimension at usage grain:
// a Top-5 ranking (click a row to narrow the shared project filter) with the
// unknown bucket rendered hatched like the "others" aggregate, plus a data
// quality card quantifying the unknown share. All numbers flow the metric
// DSL; shares divide by the ranking's all-bucket total (项目维度内占比).

import { useTranslation } from "react-i18next"
import { useProjectUsageQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { type FilterState, patchFilter } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardContent,
  CardFooter,
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
      <div className="grid gap-3 min-[1080px]:grid-cols-12">
        <Card interactive className="min-[1080px]:col-span-8">
          <CardHeader>
            <CardTitle>{t("usage.projects.rankTitle")}</CardTitle>
            <span className="text-muted-foreground/70 self-end text-xs">
              {t("usage.projects.topN", { n: TOP_N })}
            </span>
          </CardHeader>
          <CardContent className="flex flex-col gap-1.5">
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
                name={t("usage.control.unknownProject")}
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
                  formatMetricSeg(
                    t("usage.metric.cost"),
                    formatCost(rest.cost),
                  ),
                ])}
              />
            ) : null}
          </CardContent>
          <CardFooter className="text-muted-foreground/70 gap-2 text-[10.5px]">
            <span>{t("usage.projects.shareNote")}</span>
          </CardFooter>
        </Card>

        <Card interactive className="min-[1080px]:col-span-4">
          <CardHeader>
            <CardTitle>{t("usage.projects.qualityTitle")}</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-center gap-3">
            <div>
              <div className="text-2xl font-semibold tabular-nums">
                {ranking.unknown
                  ? formatSegValue(
                      formatTokens(ranking.unknown.total_tokens),
                      ranking.unknown.total_tokens / ranking.totalTokens,
                    )
                  : t("usage.projects.noUnknown")}
              </div>
              <div className="text-muted-foreground mt-1 text-[11px]">
                {t("usage.projects.unknownLabel")}
              </div>
            </div>
            <div className="flex flex-col">
              <KRow label={t("usage.projects.top3")}>
                {ranking.top3Share != null ? formatPct(ranking.top3Share) : "—"}
              </KRow>
              {ranking.rest ? (
                <KRow label={t("usage.projects.otherProjects")}>
                  {formatCount(ranking.rest.count)}
                </KRow>
              ) : null}
            </div>
          </CardContent>
          <CardFooter className="text-muted-foreground/70 text-[10.5px]">
            <span>{t("usage.projects.qualityNote")}</span>
          </CardFooter>
        </Card>
      </div>
    </QueryState>
  )
}

function KRow({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="border-border/60 flex items-baseline justify-between gap-2 border-t py-1.5 text-xs first:border-t-0 first:pt-0">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-semibold tabular-nums">{children}</span>
    </div>
  )
}
