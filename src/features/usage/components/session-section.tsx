// Session section (#106) — the dashboard's session dimension at usage grain:
// the 1–3 / 4–8 / 9–16 / 17+ turn distribution as a half-ring composition
// (#119 档位类形态：进度条 → 半环，弧段 + 环心合计，右侧行保精确读数) and
// the Top-5 sessions with a tokens/cost metric switch（cost 档即「最贵会话
// Top-N」，排序口径沿 ccusage session 报表的默认成本排序）。Top 行走系统的
// DistRow：主值 = 所选指标，sub 行铺成本/请求数/四桶分项（#119 维度行字段
// 补全）。Turn buckets partition the sessions the section lists; the band
// boundaries live in derive.sessionSectionStats (testable), and the turn
// grain's facet caveat (model/source don't apply to turns) is the backend's
// documented caliber, not re-derived here.

import { ArrowRight } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useSessionUsageQuery } from "@/app/store/api"
import { useAppDispatch } from "@/app/store/hooks"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { setView } from "@/app/store/slices/viewSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { SemicircleChart } from "@/features/usage/components/semicircle-chart"
import {
  type SessionTopMetric,
  sessionSectionStats,
} from "@/features/usage/derive"
import {
  formatCost,
  formatCount,
  formatMetricLine,
  formatMetricSeg,
  formatSegValue,
  formatTokens,
} from "@/lib/format"
import { BUCKET_DISPLAY } from "@/lib/token-buckets"
import { DistRow } from "./dist-row"

const TOP_N = 5

export function SessionSection({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const { data: rows = [], isLoading, error } = useSessionUsageQuery(filter)
  // 默认按 tokens 展示（token-first cockpit），开关同样 tokens 在前——
  // 与模型分布卡同一惯例。
  const [metric, setMetric] = useState<SessionTopMetric>("tokens")
  const stats = sessionSectionStats(rows, TOP_N, metric)
  const bandLabels = [
    t("usage.sessions.turnBand1"),
    t("usage.sessions.turnBand2"),
    t("usage.sessions.turnBand3"),
    t("usage.sessions.turnBand4"),
  ]
  // Top 行 sub 的四桶段由展示名册投影（序 = 名册序、文案键尾段同源），取数
  // 用桶键，不手抄字段名。
  const bucketSegs = BUCKET_DISPLAY.map((b) => ({
    bucket: b.bucket,
    label: t(`usage.tokens.${b.suffix}`),
  }))
  const fmt = metric === "cost" ? formatCost : formatTokens

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={rows.length === 0}
      emptyLabel={t("usage.sessions.empty")}
      emptyDescription={t("usage.sessions.emptyDesc")}
    >
      <div className="grid gap-3 min-[1080px]:grid-cols-12">
        <Card interactive className="min-[1080px]:col-span-5">
          <CardHeader>
            <CardTitle>{t("usage.sessions.turnDistTitle")}</CardTitle>
            <span className="text-muted-foreground self-end text-xs">
              {t("usage.sessions.bySessionCount")}
            </span>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-center">
            <SemicircleChart
              tiers={stats.turnBuckets.map((count, i) => ({
                label: bandLabels[i],
                count,
              }))}
              centerValue={formatCount(stats.sessions)}
              centerLabel={t("usage.kpi.sessions")}
              formatValue={formatCount}
            />
          </CardContent>
        </Card>

        <Card interactive className="min-[1080px]:col-span-7">
          <CardHeader>
            <CardTitle>{t("usage.sessions.topTitle")}</CardTitle>
            <span className="text-muted-foreground self-end text-xs">
              {metric === "cost"
                ? t("usage.sessions.topByCost", { n: TOP_N })
                : t("usage.sessions.topByTokens", { n: TOP_N })}
            </span>
            <CardAction>
              <div className="flex items-center gap-2">
                {/* tokens/cost 开关 —— 胶囊惯例同 model-distribution。 */}
                <div className="bg-muted/60 inline-flex items-center gap-0.5 rounded-md p-0.5">
                  {(["tokens", "cost"] as const).map((m) => (
                    <button
                      key={m}
                      type="button"
                      onClick={() => setMetric(m)}
                      className={`rounded-[5px] px-2 py-0.5 text-xs font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/40 ${
                        metric === m
                          ? "bg-accent-tint text-accent-brand-strong shadow-sm"
                          : "text-muted-foreground hover:text-foreground"
                      }`}
                    >
                      {m === "tokens"
                        ? t("usage.sessions.byTokens")
                        : t("usage.sessions.byCost")}
                    </button>
                  ))}
                </div>
                <button
                  type="button"
                  onClick={() => dispatch(setView("sessions"))}
                  className="text-primary hover:text-primary/80 inline-flex items-center gap-1 text-xs"
                >
                  {t("usage.sessions.allLink")}
                  <ArrowRight className="size-3" />
                </button>
              </div>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-center gap-1">
            {stats.top.map((r) => {
              const value = metric === "cost" ? r.cost : r.tokens
              return (
                <DistRow
                  key={`${r.device_id}/${r.session_id}`}
                  name={r.title}
                  value={formatSegValue(fmt(value), r.share)}
                  share={r.share}
                  sub={formatMetricLine([
                    formatMetricSeg(t("usage.metric.cost"), formatCost(r.cost)),
                    formatMetricSeg(
                      t("usage.hero.requests"),
                      formatCount(r.requests),
                    ),
                    ...bucketSegs.map((b) =>
                      formatMetricSeg(
                        b.label,
                        formatTokens(r.buckets[b.bucket]),
                      ),
                    ),
                  ])}
                />
              )
            })}
          </CardContent>
        </Card>
      </div>
    </QueryState>
  )
}
