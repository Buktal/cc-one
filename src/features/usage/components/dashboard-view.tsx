// Dashboard view (#106, 定稿 variant-c-v2: 概览首屏 + 维度分区) —
// 概览 (总消耗 + KPI 带 + 趋势 + 模型分布) → 项目 → 会话 → 请求 → 近期请求,
// each a section with an indexed head carrying a summary line (secondary
// aggregates only — never a repeat of the cards' big numbers). The sticky tab
// bar holds the section tabs (scrollspy-highlighted), the shared filter bar
// (time / source / model / project / device + reset) on its right end, and
// the page scroll progress line on its bottom edge. Single frozen layer by
// design: only the tab bar sticks — the filter row can wrap to a second line
// at narrower widths, so a second frozen layer under it would need measured
// geometry; section heads stay in flow and the tab highlight already says
// where you are. 设备分区 (#107) lands as another section/tab when its
// endpoint ships.

import dayjs from "dayjs"
import { RotateCcw } from "lucide-react"
import { useRef } from "react"
import { useTranslation } from "react-i18next"
import {
  useProjectUsageQuery,
  useSessionUsageQuery,
  useStatsQuery,
  useTrendQuery,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  type FilterState,
  patchFilter,
  resetFilter,
} from "@/app/store/slices/filterSlice"
import { Button } from "@/components/ui/button"
import { sessionSpan, spanLabelKey } from "@/features/sessions/derive"
import {
  projectRanking,
  requestHeadline,
  sessionSectionStats,
  windowDayCount,
} from "@/features/usage/derive"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import {
  formatCount,
  formatDay,
  formatInt,
  formatPct,
  formatRatio,
} from "@/lib/format"
import { cn } from "@/lib/utils"
import type { TrendBucket } from "@/types/generated/bindings"
import { useSectionScroll } from "../use-section-scroll"
import { ControlBar } from "./control-bar"
import { KpiBand } from "./kpi-band"
import { ModelDistribution } from "./model-distribution"
import { ProjectSection } from "./project-section"
import { RecentRequests } from "./recent-requests"
import { RequestSection } from "./request-section"
import { SessionSection } from "./session-section"
import { TokenHero } from "./token-hero"
import { UsageTrendChart } from "./usage-trend-chart"

/** Section registry — tab order IS scroll order; ids anchor the spy + jumps. */
const SECTIONS = [
  { id: "dash-overview", key: "usage.sections.overview" },
  { id: "dash-projects", key: "usage.sections.projects" },
  { id: "dash-sessions", key: "usage.sections.sessions" },
  { id: "dash-requests", key: "usage.sections.requests" },
  { id: "dash-recent", key: "usage.sections.recent" },
] as const
const SECTION_IDS: readonly string[] = SECTIONS.map((s) => s.id)

/** Anchor jumps must clear the frozen tab bar — the section's scroll margin
 *  matches the bar's height (single frozen layer, ~44px) plus breathing room. */
const SCROLL_MARGIN = "scroll-mt-16"

export function DashboardView() {
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  const rootRef = useRef<HTMLDivElement>(null)
  const { activeId, progress } = useSectionScroll(rootRef, SECTION_IDS)
  const { t } = useTranslation()
  const summaries = useSectionSummaries(filter)

  const jump = (id: string) => {
    document.getElementById(id)?.scrollIntoView({
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
        ? "auto"
        : "smooth",
    })
  }

  return (
    <div
      ref={rootRef}
      className="mx-auto flex w-full max-w-[1380px] flex-col gap-3 pb-4"
    >
      {/* Sticky tab bar: section tabs + shared filters + reset + progress. */}
      <div className="bg-card/90 supports-[backdrop-filter]:bg-card/75 sticky top-0 z-20 flex flex-wrap items-center gap-x-3 gap-y-2 rounded-lg border p-2 shadow-sm backdrop-blur">
        <nav
          aria-label={t("usage.sections.aria")}
          className="flex flex-wrap items-center gap-0.5"
        >
          {SECTIONS.map((s, i) => (
            <button
              key={s.id}
              type="button"
              aria-current={activeId === s.id ? "true" : undefined}
              onClick={() => jump(s.id)}
              className={cn(
                "focus-visible:ring-ring/40 rounded-md px-2.5 py-1 text-xs font-medium whitespace-nowrap transition-colors outline-none focus-visible:ring-2",
                activeId === s.id
                  ? "bg-accent-tint text-accent-brand-strong"
                  : "text-muted-foreground hover:bg-hover hover:text-foreground",
              )}
            >
              <span className="text-muted-foreground/60 mr-1 tabular-nums">
                {String(i + 1).padStart(2, "0")}
              </span>
              {summaries.label(s.key)}
            </button>
          ))}
        </nav>
        <div className="ml-auto flex min-w-0 flex-wrap items-center gap-2">
          <ControlBar />
          <Button
            variant="ghost"
            size="sm"
            className="h-8 px-2.5 text-xs"
            onClick={() => dispatch(resetFilter())}
          >
            <RotateCcw className="size-3.5" />
            {summaries.resetLabel}
          </Button>
        </div>
        {/* Page scroll progress line hugging the bar's bottom edge. */}
        <div
          aria-hidden="true"
          className="absolute right-2 bottom-0 left-2 h-0.5 overflow-hidden rounded-full"
        >
          <div
            className="bg-primary h-full rounded-full opacity-75"
            style={{ width: `${progress * 100}%` }}
          />
        </div>
      </div>

      <Section
        id="dash-overview"
        index={1}
        title={summaries.label("usage.sections.overview")}
        summary={summaries.overview}
      >
        <div className="grid gap-3 min-[1080px]:grid-cols-12">
          <div className="min-[1080px]:col-span-4">
            <TokenHero filter={filter} />
          </div>
          <div className="min-[1080px]:col-span-8">
            <KpiBand filter={filter} />
          </div>
          <div className="min-[1080px]:col-span-8">
            <UsageTrendChart filter={filter} />
          </div>
          <div className="min-[1080px]:col-span-4">
            <ModelDistribution
              filter={filter}
              onPickModel={(m) => dispatch(patchFilter({ model: m }))}
              onClearModel={() => dispatch(patchFilter({ model: "" }))}
            />
          </div>
        </div>
      </Section>

      <Section
        id="dash-projects"
        index={2}
        title={summaries.label("usage.sections.projects")}
        summary={summaries.projects}
      >
        <ProjectSection filter={filter} />
      </Section>

      <Section
        id="dash-sessions"
        index={3}
        title={summaries.label("usage.sections.sessions")}
        summary={summaries.sessions}
      >
        <SessionSection filter={filter} />
      </Section>

      <Section
        id="dash-requests"
        index={4}
        title={summaries.label("usage.sections.requests")}
        summary={summaries.requests}
      >
        <RequestSection filter={filter} />
      </Section>

      <Section
        id="dash-recent"
        index={5}
        title={summaries.label("usage.sections.recent")}
        summary={summaries.recent}
      >
        <RecentRequests />
      </Section>
    </div>
  )
}

/** One dashboard section: indexed head (title + secondary-aggregate summary)
 *  + the section body. Heads stay in flow (single frozen layer — see the file
 *  header); the scroll margin clears the frozen tab bar for anchor jumps. */
function Section({
  id,
  index,
  title,
  summary,
  children,
}: {
  id: string
  index: number
  title: string
  summary: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section id={id} className={cn("flex flex-col gap-2.5", SCROLL_MARGIN)}>
      <header className="bg-card/60 flex flex-wrap items-baseline gap-x-2.5 rounded-md border px-2.5 py-1.5 backdrop-blur">
        <span className="text-muted-foreground/60 text-[10px] font-semibold tracking-wider tabular-nums">
          {String(index).padStart(2, "0")}
        </span>
        <h3 className="text-[13px] font-semibold whitespace-nowrap">{title}</h3>
        <span className="text-muted-foreground min-w-0 text-[11px] tabular-nums">
          {summary}
        </span>
      </header>
      {children}
    </section>
  )
}

/**
 * The section heads' summary lines + the tab labels. Reads the SAME queries
 * the section bodies consume (one cache entry per filter) and derives only
 * secondary aggregates — per the acceptance rule the summary never repeats a
 * card's big number. Returns a tiny `label(key)` closure so the registry's
 * i18n keys resolve through one path.
 */
function useSectionSummaries(filter: FilterState) {
  const { t } = useTranslation()
  const { from_day, to_day } = effectiveDays(filter)
  const days = windowDayCount(from_day, to_day)
  const { data: stats } = useStatsQuery(filter)
  const { data: projectRows = [] } = useProjectUsageQuery(filter)
  const { data: sessionRows = [] } = useSessionUsageQuery(filter)
  const ranking = projectRanking(projectRows, 0)
  const sessionStats = sessionSectionStats(sessionRows, 0)

  // The requests summary needs the per-bucket counts — same trend query (and
  // bucket rule) the requests section's bars read.
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  // Single local-day check mirrors the trend chart: a UTC+8 "today" maps to a
  // 24h UTC window that still falls on one local day.
  const singleDay = !!fromTs && !!toTs && dayjs(fromTs).isSame(toTs, "day")
  const bucket: TrendBucket = singleDay ? "Hour" : "Day"
  const { data: trend = [] } = useTrendQuery({ filter, bucket })
  const headline = requestHeadline(trend, days)

  const overview =
    days == null
      ? t("usage.control.allTime")
      : t("usage.sections.windowSum", {
          from: formatDay(from_day),
          to: formatDay(to_day),
          n: days,
        })
  const projects = t("usage.sections.projectsSum", {
    n: formatCount(ranking.knownCount),
    top3: ranking.top3Share != null ? formatPct(ranking.top3Share) : "—",
    unknown:
      ranking.unknown != null
        ? t("usage.sections.unknownSum", {
            pct: formatPct(ranking.unknown.total_tokens / ranking.totalTokens),
          })
        : "",
  })
  const longest =
    sessionStats.longestSpanMs != null
      ? spanLabelKey(sessionSpan(sessionStats.longestSpanMs))
      : null
  const sessions = t("usage.sections.sessionsSum", {
    n: formatCount(sessionStats.sessions),
    turns:
      sessionStats.avgTurns != null ? formatRatio(sessionStats.avgTurns) : "—",
    longest: longest ? t(longest.key, longest.vars) : "—",
  })
  const requests = t("usage.sections.requestsSum", {
    n: formatCount(stats?.request_count ?? 0),
    avg: headline.dailyAvg != null ? formatCount(headline.dailyAvg) : "—",
    peak: headline.peakCount != null ? formatInt(headline.peakCount) : "—",
  })

  return {
    label: (key: string) => t(key),
    resetLabel: t("usage.control.reset"),
    overview,
    projects,
    sessions,
    requests,
    recent: t("usage.sections.recentSum"),
  }
}
