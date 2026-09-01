// 会话排行卡（#119 四期重排：原 session-section 的 Top-N 卡拆出独立成卡）。
// 维度排行组的第二位（模型分布 > 会话 > 项目 > 设备）：Top-5 会话按
// tokens/cost 切换（cost 档即「最贵会话 Top-N」，排序口径沿 ccusage session
// 报表的默认成本排序），头部链接跳会话视图看全量。行走系统的 DistRow：主值
// = 所选指标，sub 行铺成本/请求数/四桶分项。

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

export function SessionRanking({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const { data: rows = [], isLoading, error } = useSessionUsageQuery(filter)
  // 默认按 tokens 展示（token-first cockpit），开关同样 tokens 在前——
  // 与模型分布卡同一惯例。
  const [metric, setMetric] = useState<SessionTopMetric>("tokens")
  const stats = sessionSectionStats(rows, TOP_N, metric)
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
      {/* 卡体由父级网格定位（维度排行组内与模型分布同行）。 */}
      <Card interactive className="h-full">
        <CardHeader>
          <CardTitle>{t("usage.sessions.topTitle")}</CardTitle>
          <CardAction>
            <div className="flex items-center gap-2">
              {/* 副标进 CardAction（全页 header 单行制）——指标读数与开关
                  同行，开关切换时副标同步换口径。 */}
              <span className="text-muted-foreground text-xs tabular-nums">
                {metric === "cost"
                  ? t("usage.sessions.topByCost", { n: TOP_N })
                  : t("usage.sessions.topByTokens", { n: TOP_N })}
              </span>
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
                    formatMetricSeg(b.label, formatTokens(r.buckets[b.bucket])),
                  ),
                ])}
              />
            )
          })}
        </CardContent>
      </Card>
    </QueryState>
  )
}
