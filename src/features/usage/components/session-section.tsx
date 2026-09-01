// Session section (#106) — the dashboard's session dimension at usage grain:
// the 1–3 / 4–8 / 9–16 / 17+ turn distribution as a half-ring composition
// (#119 档位类形态：进度条 → 半环，弧段 + 环心合计，右侧行保精确读数) and
// the Top-5 sessions by tokens (占比为占总消耗). Turn buckets partition the
// sessions the section lists; the band boundaries live in
// derive.sessionSectionStats (testable), and the turn grain's facet caveat
// (model/source don't apply to turns) is the backend's documented caliber,
// not re-derived here.

import { ArrowRight } from "lucide-react"
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
import { sessionSectionStats } from "@/features/usage/derive"
import { formatCount, formatSegValue, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"

const TOP_N = 5

export function SessionSection({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const { data: rows = [], isLoading, error } = useSessionUsageQuery(filter)
  const stats = sessionSectionStats(rows, TOP_N)
  const bandLabels = [
    t("usage.sessions.turnBand1"),
    t("usage.sessions.turnBand2"),
    t("usage.sessions.turnBand3"),
    t("usage.sessions.turnBand4"),
  ]

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
              {t("usage.sessions.topByTokens", { n: TOP_N })}
            </span>
            <CardAction>
              <button
                type="button"
                onClick={() => dispatch(setView("sessions"))}
                className="text-primary hover:text-primary/80 inline-flex items-center gap-1 text-xs"
              >
                {t("usage.sessions.allLink")}
                <ArrowRight className="size-3" />
              </button>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-1 flex-col justify-center">
            {stats.top.map((r, i) => (
              <div
                key={`${r.device_id}/${r.session_id}`}
                className={cn(
                  "flex items-baseline justify-between gap-3 py-2 text-xs",
                  i > 0 && "border-border/60 border-t",
                )}
              >
                <span className="min-w-0 truncate font-medium">{r.title}</span>
                <span className="text-muted-foreground shrink-0 tabular-nums">
                  {formatSegValue(formatTokens(r.tokens), r.share)}
                </span>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </QueryState>
  )
}
