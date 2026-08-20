// KPI band — the overview's second card (#106): two equal-height rows of
// KPI cells beside the TokenHero. Main row (5): 平均时长 / 请求·轮 / 会话 /
// 项目 / 设备; secondary row (4): 命中率 / 成本 / 日均 Token / 最长会话 —
// cost and hit rate promoted to second-tier KPIs per the #96 decision
// (variant-c-v2). Cell = big number + label + sub line; every number flows
// through the metric DSL. The token delta/daily-average caliber comes from
// use-token-snapshot (shared with the hero); the 会话/项目/设备 cells read the
// SAME projectUsage / sessionUsage queries the sections below consume (one
// cache entry per filter).

import { useTranslation } from "react-i18next"
import { useProjectUsageQuery, useSessionUsageQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { sessionSpan, spanLabelKey } from "@/features/sessions/derive"
import {
  projectRanking,
  sessionSectionStats,
  windowDayCount,
} from "@/features/usage/derive"
import { useTokenSnapshot } from "@/features/usage/use-token-snapshot"
import { effectiveDays } from "@/lib/date-range"
import {
  formatCost,
  formatCount,
  formatDuration,
  formatPct,
  formatRatio,
  formatTokens,
} from "@/lib/format"
import { cn } from "@/lib/utils"

interface Cell {
  key: string
  value: string
  label: string
  sub?: string
  accent?: "cost" | "accent"
}

export function KpiBand({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const {
    stats: s,
    deltaPct,
    singleDay,
    dailyAvg,
    hourlyAvg,
  } = useTokenSnapshot(filter)
  const { data: projectRows = [] } = useProjectUsageQuery(filter)
  const { data: sessionRows = [] } = useSessionUsageQuery(filter)
  // topN=0: the ranking itself renders in the project section below — here
  // only the aggregates (known count / Top3 concentration) are read.
  const ranking = projectRanking(projectRows, 0)
  const sessions = sessionSectionStats(sessionRows, 0)
  const { from_day, to_day } = effectiveDays(filter)
  const days = windowDayCount(from_day, to_day)

  // 无轮次时无可算比率 — 占位破折号（与 formatDuration 的空值语义一致）。
  const perTurn =
    s.turn_count > 0 ? formatRatio(s.request_count / s.turn_count) : "—"
  const longest = sessions.longestSpanMs
    ? spanLabelKey(sessionSpan(sessions.longestSpanMs))
    : null

  const main: Cell[] = [
    {
      key: "avgDuration",
      value: formatDuration(s.avg_turn_duration_ms),
      label: t("usage.kpi.avgDuration"),
      sub: t("usage.kpi.p95", {
        v: formatDuration(s.p95_turn_duration_ms),
      }),
    },
    {
      key: "perTurn",
      value: perTurn,
      label: t("usage.kpi.perTurn"),
      sub: `${t("usage.hero.requests")} ${formatCount(s.request_count)}`,
    },
    {
      key: "sessions",
      value: formatCount(sessions.sessions),
      label: t("usage.kpi.sessions"),
      sub:
        sessions.subagentShare != null
          ? t("usage.kpi.subagentShare", {
              pct: formatPct(sessions.subagentShare),
            })
          : undefined,
    },
    {
      key: "projects",
      value: formatCount(ranking.knownCount),
      label: t("usage.kpi.projects"),
      sub:
        ranking.top3Share != null
          ? t("usage.kpi.top3Share", { pct: formatPct(ranking.top3Share) })
          : undefined,
    },
    {
      key: "devices",
      value: formatCount(sessions.devices),
      label: t("usage.kpi.devices"),
      sub:
        sessions.topDeviceShare != null
          ? t("usage.kpi.topDevice", {
              pct: formatPct(sessions.topDeviceShare),
            })
          : undefined,
    },
  ]

  const secondary: Cell[] = [
    {
      key: "hitRate",
      value: formatPct(s.cache_hit_rate),
      label: t("usage.hero.cacheHitRate"),
      sub: t("usage.kpi.hitSub", { v: formatTokens(s.cache_read_tokens) }),
      accent: "accent",
    },
    {
      key: "cost",
      value: formatCost(s.total_cost_usd),
      label: t("usage.kpi.totalCost"),
      sub:
        days != null
          ? t("usage.kpi.dailyCost", {
              v: formatCost(s.total_cost_usd / days),
            })
          : undefined,
      accent: "cost",
    },
    {
      key: "dailyTokens",
      // 单日窗口的「日均」退化为今日总量，语义失真 — 改显小时均（与 hero
      // 的口径切换一致），标签跟随。
      value: formatTokens(singleDay ? hourlyAvg : dailyAvg),
      label: singleDay
        ? t("usage.kpi.hourlyTokens")
        : t("usage.kpi.dailyTokens"),
      sub:
        deltaPct != null
          ? t("usage.kpi.deltaSub", {
              dir: deltaPct >= 0 ? "↑" : "↓",
              pct: formatPct(Math.abs(deltaPct)),
              vs: singleDay
                ? t("usage.hero.vsYesterday")
                : t("usage.hero.vsStart"),
            })
          : undefined,
    },
    {
      key: "longest",
      value: longest
        ? t(longest.key, longest.vars)
        : t("usage.kpi.longestNone"),
      label: t("usage.kpi.longestSession"),
      sub:
        sessions.avgTurns != null
          ? t("usage.kpi.avgTurnsSub", {
              v: formatRatio(sessions.avgTurns),
            })
          : undefined,
    },
  ]

  return (
    <Card interactive className="h-full">
      <CardHeader>
        <CardTitle>{t("usage.kpi.title")}</CardTitle>
      </CardHeader>
      <CardContent className="flex h-full flex-1 flex-col justify-center gap-3">
        <div className="@container">
          <div className="grid grid-cols-2 gap-x-4 gap-y-3 @[34rem]:grid-cols-3 @[54rem]:grid-cols-5">
            {main.map((c, i) => (
              <KpiCell key={c.key} cell={c} first={i === 0} />
            ))}
          </div>
          <div className="border-border/60 mt-3 grid grid-cols-2 gap-x-4 gap-y-3 border-t pt-3 @[34rem]:grid-cols-4">
            {secondary.map((c, i) => (
              <KpiCell key={c.key} cell={c} first={i === 0} />
            ))}
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function KpiCell({ cell, first }: { cell: Cell; first: boolean }) {
  return (
    <div
      className={cn(
        "@[34rem]:border-border/60 min-w-0",
        // 容器 ≥34rem 时竖分隔线（首列无线）；以下靠行距分格。
        !first && "@[34rem]:border-l @[34rem]:pl-4",
      )}
    >
      <div
        className="text-xl leading-tight font-semibold tabular-nums"
        style={
          cell.accent === "cost"
            ? { color: "var(--metric-cost)" }
            : cell.accent === "accent"
              ? { color: "var(--primary)" }
              : undefined
        }
      >
        {cell.value}
      </div>
      <div className="text-muted-foreground mt-1 text-[11px] whitespace-nowrap">
        {cell.label}
      </div>
      {cell.sub ? (
        <div className="text-muted-foreground/70 mt-0.5 truncate text-[10.5px] tabular-nums">
          {cell.sub}
        </div>
      ) : null}
    </div>
  )
}
