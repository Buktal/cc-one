// RecentRequests — dashboard middle-column footer. Latest N request
// rows as a compact list + a "全部 →" link into the logs view. Doubles as a
// height-filler so the middle column tracks the right column, and as a quick
// path from the dashboard into the full ledger. Polls with the shared interval.

import { ArrowRight } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useCountQuery, useLogsQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { setView } from "@/app/store/slices/viewSlice"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { formatCost, formatInt, formatTime } from "@/lib/format"
import { tokenTotal } from "@/lib/usage"
import { cn } from "@/lib/utils"

import { useDeviceLabelMap } from "../use-device-options"

const LIMIT = 5

export function RecentRequests() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  const deviceLabel = useDeviceLabelMap()
  const { data: rows = [] } = useLogsQuery({
    filter,
    limit: LIMIT,
    offset: 0,
  })
  const { data: total = 0 } = useCountQuery(filter)

  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.recent.title")}</CardTitle>
        <CardAction>
          <button
            type="button"
            onClick={() => dispatch(setView("logs"))}
            className="text-primary hover:text-primary/80 inline-flex items-center gap-1 text-xs"
          >
            {t("usage.recent.all")}
            <ArrowRight className="size-3" />
            {total > 0 ? (
              <span className="text-muted-foreground">
                ({formatInt(total)})
              </span>
            ) : null}
          </button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col">
        {rows.length === 0 ? (
          <span className="text-muted-foreground py-6 text-center text-xs">
            {t("usage.recent.empty")}
          </span>
        ) : (
          rows.map((r, i) => (
            <div
              key={r.uuid}
              className={cn(
                "flex items-center gap-2 py-2.5",
                i > 0 && "border-border/60 border-t",
              )}
            >
              <span className="truncate font-mono text-xs font-medium">
                {r.model}
              </span>
              <div className="ml-auto flex shrink-0 items-center gap-3">
                <span className="text-foreground text-sm font-semibold tabular-nums">
                  {formatInt(tokenTotal(r))}
                  <span className="text-muted-foreground ml-1 text-[10px] font-normal">
                    tok
                  </span>
                </span>
                <span className="text-muted-foreground flex items-center gap-2 text-[11px] tabular-nums">
                  <span>
                    {t("usage.recent.in", { n: formatInt(r.tokens.input) })}
                  </span>
                  <span aria-hidden="true">·</span>
                  <span>
                    {t("usage.recent.out", { n: formatInt(r.tokens.output) })}
                  </span>
                  <span aria-hidden="true">·</span>
                  <span>{formatCost(r.total_cost_usd)}</span>
                  <span aria-hidden="true">·</span>
                  <span>{formatTime(r.timestamp)}</span>
                  {deviceLabel.size > 0 ? (
                    <>
                      <span aria-hidden="true">·</span>
                      <span className="max-w-24 truncate">
                        {deviceLabel.get(r.device_id) ??
                          r.device_id.slice(0, 8)}
                      </span>
                    </>
                  ) : null}
                </span>
              </div>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  )
}
