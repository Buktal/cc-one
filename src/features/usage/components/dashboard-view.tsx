// Dashboard view (#106, 定稿 variant-c-v2: 概览首屏 + 维度分区) —
// 概览 (总消耗 + KPI 带 + 趋势 + 模型分布) → 设备 → 项目 → 会话 → 请求 →
// 近期请求, each a section with an indexed head carrying a summary line
// (secondary aggregates only — never a repeat of the cards' big numbers). The
// sticky layer is the section tabs only (scrollspy-highlighted, page scroll
// progress line on its bottom edge); the shared filter bar (time / source /
// model / project / device + reset) sits in flow above the sections — the
// same shape as the logs view's header. Single frozen
// layer by design: only the tab bar sticks; the filter row can wrap to a
// second line at narrower widths, so the bar's height is MEASURED
// (ResizeObserver) — anchor scroll margins and the scrollspy edge both track
// the real bar, never a guessed constant. The bar is a flat solid card
// surface (index.css 平面铁律: no glass, no backdrop-blur).

import dayjs from "dayjs"
import { RotateCcw } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useDevicesQuery,
  useDeviceUsageQuery,
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
  deviceSectionStats,
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
import { DeviceSection } from "./device-section"
import { KpiBand } from "./kpi-band"
import { ModelDistribution } from "./model-distribution"
import { ProjectSection } from "./project-section"
import { RecentRequests } from "./recent-requests"
import { RequestSection } from "./request-section"
import { SessionSection } from "./session-section"
import { TokenHero } from "./token-hero"
import { UsageTrendChart } from "./usage-trend-chart"

/** Section registry — tab order IS scroll order; ids anchor the spy + jumps.
 *  设备 leads the dimension sections (the #96 prototype's order). */
const SECTIONS = [
  { id: "dash-overview", key: "usage.sections.overview" },
  { id: "dash-devices", key: "usage.sections.devices" },
  { id: "dash-projects", key: "usage.sections.projects" },
  { id: "dash-sessions", key: "usage.sections.sessions" },
  { id: "dash-requests", key: "usage.sections.requests" },
  { id: "dash-recent", key: "usage.sections.recent" },
] as const
const SECTION_IDS: readonly string[] = SECTIONS.map((s) => s.id)

export function DashboardView() {
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  const rootRef = useRef<HTMLDivElement>(null)
  const barRef = useRef<HTMLDivElement>(null)
  // 吸顶栏实测高度（折行时变两行）。锚点 scroll-margin 与 scrollspy 边缘都
  // 从它派生 —— 不变量「让位 ≥ 吸顶高度」由测量守住，不靠估计常量。
  // 初值 48 = 单行形态（p-2 × 2 + 32px 内容），ResizeObserver 首帧即校正。
  const [stickyBarH, setStickyBarH] = useState(48)
  useEffect(() => {
    const bar = barRef.current
    if (!bar) return
    const ro = new ResizeObserver(() => setStickyBarH(bar.offsetHeight))
    ro.observe(bar)
    return () => ro.disconnect()
  }, [])
  const { activeId, progress } = useSectionScroll(
    rootRef,
    SECTION_IDS,
    stickyBarH,
  )
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
      {/* Sticky tabs — 吸顶只留 section tabs 单行（scrollspy 高亮 + 底边
          进度线）。筛选不吸顶：与其他视图（日志页 / 会话工作台）一致，作
          为页面顶部的常规 in-flow 行随页滚动。Flat solid bg-card（平面铁律）。 */}
      <div
        ref={barRef}
        className="bg-card sticky top-0 z-20 rounded-lg border p-2 shadow-sm"
      >
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

      {/* 筛选行 —— 与日志页同形（整宽 in-flow，不吸顶）。flex-1 包裹给
          ControlBar 确定宽度：其根节点是 @container（inline-size
          containment，固有宽度为 0），不能直接当行向 flex 子项用，契约见
          control-bar.tsx。重置右贴行尾。 */}
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <div className="min-w-0 flex-1">
          <ControlBar />
        </div>
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

      <Section
        id="dash-overview"
        index={1}
        title={summaries.label("usage.sections.overview")}
        summary={summaries.overview}
        stickyBarH={stickyBarH}
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
        id="dash-devices"
        index={2}
        title={summaries.label("usage.sections.devices")}
        summary={summaries.devices}
        stickyBarH={stickyBarH}
      >
        <DeviceSection filter={filter} />
      </Section>

      <Section
        id="dash-projects"
        index={3}
        title={summaries.label("usage.sections.projects")}
        summary={summaries.projects}
        stickyBarH={stickyBarH}
      >
        <ProjectSection filter={filter} />
      </Section>

      <Section
        id="dash-sessions"
        index={4}
        title={summaries.label("usage.sections.sessions")}
        summary={summaries.sessions}
        stickyBarH={stickyBarH}
      >
        <SessionSection filter={filter} />
      </Section>

      <Section
        id="dash-requests"
        index={5}
        title={summaries.label("usage.sections.requests")}
        summary={summaries.requests}
        stickyBarH={stickyBarH}
      >
        <RequestSection filter={filter} />
      </Section>

      <Section
        id="dash-recent"
        index={6}
        title={summaries.label("usage.sections.recent")}
        summary={summaries.recent}
        stickyBarH={stickyBarH}
      >
        <RecentRequests />
      </Section>
    </div>
  )
}

/** One dashboard section: indexed head (title + secondary-aggregate summary)
 *  + the section body. Heads stay in flow (single frozen layer — see the file
 *  header); the scroll margin clears the frozen tab bar's MEASURED height
 *  (plus a small breathing gap) for anchor jumps. */
function Section({
  id,
  index,
  title,
  summary,
  stickyBarH,
  children,
}: {
  id: string
  index: number
  title: string
  summary: React.ReactNode
  stickyBarH: number
  children: React.ReactNode
}) {
  return (
    <section
      id={id}
      className="flex flex-col gap-2.5"
      style={{ scrollMarginTop: stickyBarH + 12 }}
    >
      <header className="bg-card flex flex-wrap items-baseline gap-x-2.5 rounded-md border px-2.5 py-1.5">
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
  const { data: deviceRows = [] } = useDeviceUsageQuery(filter)
  const { data: registryDevices = [] } = useDevicesQuery()
  const ranking = projectRanking(projectRows, 0)
  const sessionStats = sessionSectionStats(sessionRows, 0)
  const deviceStats = deviceSectionStats(deviceRows)

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
  // Registry devices with no usage in the window stay invisible as rows —
  // the summary says how many, so "where's my other machine" is answered.
  const silentDevices = Math.max(
    registryDevices.length - deviceStats.devices,
    0,
  )
  const devices = t("usage.sections.devicesSum", {
    n: formatCount(deviceStats.devices),
    top: deviceStats.topShare != null ? formatPct(deviceStats.topShare) : "—",
    silent:
      silentDevices > 0
        ? t("usage.sections.devicesSilent", {
            n: formatCount(silentDevices),
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
    devices,
    projects,
    sessions,
    requests,
    recent: t("usage.sections.recentSum"),
  }
}
