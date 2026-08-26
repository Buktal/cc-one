// KPI band — the overview's second card (#106 R1 定稿), beside the TokenHero
// (4/12 + 8/12 两卡同高). Main row (5 大数字格): 平均时长 / 请求·轮 / 会话 /
// 项目 / 设备; secondary row (4 格): 命中率 / 成本 / 日均 Token / 最长会话 —
// 副行是 R1 的降级形态: 15px 值 + 单行 label (补充口径并进 label, · 分隔),
// 与主行 20px 拉开层级。两行固定列数 (5 / 4) 永不折行 — 格宽随容器伸缩,
// 小窗口靠 min-w-0 + truncate 兜底。Every number flows through the metric
// DSL. The token delta/daily-average caliber comes from use-token-snapshot
// (shared with the hero); the 会话/项目/设备 cells read the SAME
// projectUsage / sessionUsage / deviceUsage queries the sections below
// consume (one cache entry per filter).

import { useTranslation } from "react-i18next"
import {
  useDeviceUsageQuery,
  useProjectUsageQuery,
  useSessionUsageQuery,
} from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { sessionSpan, spanLabelKey } from "@/features/sessions/derive"
import {
  deviceSectionStats,
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
  const { data: deviceRows = [] } = useDeviceUsageQuery(filter)
  // topN=0: the ranking itself renders in the project section below — here
  // only the aggregates (known count / Top3 concentration) are read.
  const ranking = projectRanking(projectRows, 0)
  const sessions = sessionSectionStats(sessionRows, 0)
  // 设备 KPI 的口径 = 设备分区本身（usage 粒度的活跃设备），不再从会话行派生。
  const devices = deviceSectionStats(deviceRows)
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
      value: formatCount(devices.devices),
      label: t("usage.kpi.devices"),
      sub:
        devices.topShare != null
          ? t("usage.kpi.topDevice", {
              pct: formatPct(devices.topShare),
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
      // 的口径切换一致），标签跟随。delta 只显箭头+幅度，vs 口径已在 hero。
      value: formatTokens(singleDay ? hourlyAvg : dailyAvg),
      label: singleDay
        ? t("usage.kpi.hourlyTokens")
        : t("usage.kpi.dailyTokens"),
      sub:
        deltaPct != null
          ? `${deltaPct >= 0 ? "↑" : "↓"} ${formatPct(Math.abs(deltaPct))}`
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
      <CardContent className="flex h-full flex-1 flex-col justify-center gap-3.5">
        {/* 主行：固定 5 列不折行 — 格宽随容器伸缩，min-w-0 + truncate 兜底。 */}
        <div className="grid grid-cols-5 gap-x-4">
          {main.map((c, i) => (
            <KpiCell key={c.key} cell={c} first={i === 0} />
          ))}
        </div>
        {/* 副行：固定 4 列，降级形态（15px 值 + label 并口径单行）。 */}
        <div className="border-border/60 grid grid-cols-4 gap-x-6 border-t pt-3">
          {secondary.map((c) => (
            <SubCell key={c.key} cell={c} />
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

/** accent 色的单一映射 —— 主行格子与副行格子共用。 */
function accentStyle(accent?: "cost" | "accent") {
  return accent === "cost"
    ? { color: "var(--metric-cost)" }
    : accent === "accent"
      ? { color: "var(--primary)" }
      : undefined
}

function KpiCell({ cell, first }: { cell: Cell; first: boolean }) {
  return (
    <div
      className={cn(
        "min-w-0",
        // 竖分隔线（首列无线）在任意宽度都画 —— 行永不折行，分隔不依赖断点。
        !first && "border-border/60 border-l pl-4",
      )}
    >
      <div className="text-xl leading-tight font-semibold tabular-nums">
        <span style={accentStyle(cell.accent)}>{cell.value}</span>
      </div>
      <div className="text-muted-foreground mt-1 truncate text-[11.5px]">
        {cell.label}
      </div>
      {cell.sub ? (
        <div className="text-muted-foreground mt-0.5 truncate text-[11px] tabular-nums">
          {cell.sub}
        </div>
      ) : null}
    </div>
  )
}

function SubCell({ cell }: { cell: Cell }) {
  return (
    <div className="min-w-0">
      <div className="text-[15px] leading-tight font-semibold tabular-nums">
        <span style={accentStyle(cell.accent)}>{cell.value}</span>
      </div>
      <div className="text-muted-foreground mt-0.5 truncate text-[11px]">
        {cell.sub ? `${cell.label} · ${cell.sub}` : cell.label}
      </div>
    </div>
  )
}
