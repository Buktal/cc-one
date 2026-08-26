// Dashboard view (#106) — 概览 (总消耗 + KPI 带 + 趋势 + 模型分布) → 设备 →
// 项目 → 会话 → 请求 → 近期请求。分区不设标题横条：分区身份由吸顶 tabs
// 承载 (编号 + scrollspy 高亮 + 底边进度线)，内容卡片自带各自的标题。
// The shared filter bar (time / source / model / project / device + reset)
// sits in flow above the sections — the same shape as the logs view's
// header. Single frozen layer by design: only the tab bar sticks, so anchor
// scroll margins and the scrollspy edge derive from its MEASURED height
// (ResizeObserver — the bar can wrap at narrow widths), never a guessed
// constant. Flat solid bg-card surface (index.css 平面铁律: no glass, no
// backdrop-blur).

import { RotateCcw } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter, resetFilter } from "@/app/store/slices/filterSlice"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
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
              {t(s.key)}
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

      {/* 筛选行 —— 与日志页同形（整宽 in-flow，不吸顶）。重置右贴行尾。 */}
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
          {t("usage.control.reset")}
        </Button>
      </div>

      <Section id="dash-overview" stickyBarH={stickyBarH}>
        {/* R1 概览：Hero 4/12 + KPI 8/12 首行两卡同高，趋势 8 + 分布 4 次行。 */}
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

      <Section id="dash-devices" stickyBarH={stickyBarH}>
        <DeviceSection filter={filter} />
      </Section>

      <Section id="dash-projects" stickyBarH={stickyBarH}>
        <ProjectSection filter={filter} />
      </Section>

      <Section id="dash-sessions" stickyBarH={stickyBarH}>
        <SessionSection filter={filter} />
      </Section>

      <Section id="dash-requests" stickyBarH={stickyBarH}>
        <RequestSection filter={filter} />
      </Section>

      <Section id="dash-recent" stickyBarH={stickyBarH}>
        <RecentRequests />
      </Section>
    </div>
  )
}

/** One dashboard section — a scroll-margin-anchored block (no head; the
 *  sticky tabs carry the section identity). The margin clears the frozen
 *  tab bar's MEASURED height plus a small breathing gap for anchor jumps. */
function Section({
  id,
  stickyBarH,
  children,
}: {
  id: string
  stickyBarH: number
  children: React.ReactNode
}) {
  return (
    <section
      id={id}
      className="flex flex-col gap-2.5"
      style={{ scrollMarginTop: stickyBarH + 12 }}
    >
      {children}
    </section>
  )
}
