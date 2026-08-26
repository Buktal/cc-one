// Session section (#106) — the dashboard's session dimension at usage grain:
// the 1–3 / 4–8 / 9–16 / 17+ turn distribution (by session count, with the
// subagent share below) and the Top-5 sessions by tokens (占比为占总消耗).
// Turn buckets partition the sessions the section lists; the band boundaries
// live in derive.sessionSectionStats (testable), and the turn grain's facet
// caveat (model/source don't apply to turns) is the backend's documented
// caliber, not re-derived here.

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
          <CardContent className="flex flex-1 flex-col justify-center gap-2">
            {stats.turnBuckets.map((count, i) => (
              <BandRow
                key={bandLabels[i]}
                label={bandLabels[i]}
                share={count / (stats.sessions || 1)}
                value={formatSegValue(
                  formatCount(count),
                  count / (stats.sessions || 1),
                )}
              />
            ))}
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

/** One labeled band row: label · bar · `数量 · 占比` (session count metric). */
function BandRow({
  label,
  share,
  value,
}: {
  label: string
  share: number
  value: string
}) {
  return (
    <div className="grid grid-cols-[76px_minmax(0,1fr)_104px] items-center gap-2 text-xs">
      <span className="text-muted-foreground truncate">{label}</span>
      <div className="bg-muted h-1.5 w-full overflow-hidden rounded-full">
        <div
          className="bg-primary/70 h-full rounded-full"
          style={{ width: `${Math.max(share * 100, 2)}%` }}
        />
      </div>
      <span className="text-muted-foreground text-right tabular-nums">
        {value}
      </span>
    </div>
  )
}
