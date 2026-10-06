import { RotateCcw } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useDevicesQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  dayRangePatch,
  patchFilter,
  resetFilter,
} from "@/app/store/slices/filterSlice"
import { Button } from "@/components/ui/button"
import { CalendarHeatmap } from "@/features/usage/components/calendar-heatmap"
import { ControlBar } from "@/features/usage/components/control-bar"
import { DailyCostChart } from "@/features/usage/components/daily-cost-chart"
import { DailyRequestChart } from "@/features/usage/components/daily-request-chart"
import { DeviceSection } from "@/features/usage/components/device-section"
import { DurationDistribution } from "@/features/usage/components/duration-distribution"
import { KpiBand } from "@/features/usage/components/kpi-band"
import { ModelDistribution } from "@/features/usage/components/model-distribution"
import { ProjectSection } from "@/features/usage/components/project-section"
import { RecentRequests } from "@/features/usage/components/recent-requests"
import { SessionRanking } from "@/features/usage/components/session-ranking"
import { TokenHero } from "@/features/usage/components/token-hero"
import { TurnDistribution } from "@/features/usage/components/turn-distribution"
import { UsageTrendChart } from "@/features/usage/components/usage-trend-chart"
import { windowDayCount } from "@/features/usage/derive"
import { effectiveDays } from "@/lib/date-range"
import { cn } from "@/lib/utils"

export function DashboardView() {
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  const { t } = useTranslation()
  // 窗口日历天数是日历热力两形态（周历 / 小时矩阵）的同一事实来源（null
  // = 无界窗口，如「全部」——恒走宽窗形态）。
  const { from_day, to_day } = effectiveDays(filter)
  const spanDays = windowDayCount(from_day, to_day)
  // 「单机」判定走设备注册表（listDevices）台数，与 DeviceScopeControl /
  // DeviceSection 的行可点判定同一口径：注册表 ≤1 台 = 无切换目标、无排行
  // 可读，设备卡整个不渲染（而不是渲染一张空壳）。加载中按单机处理，
  // 多机注册表就绪后卡片出现在页面尾部，不闪在首屏路径上。
  const { data: devices = [] } = useDevicesQuery()
  const multiDevice = devices.length > 1

  return (
    <div className="dashboard-view mx-auto flex min-w-0 max-w-[1480px] flex-col gap-5 self-stretch pb-4">
      {/* 筛选行 —— 与日志页同形（整宽 in-flow）。重置右贴行尾。 */}
      <div className="dashboard-filters">
        <div className="min-w-0 flex-1">
          <ControlBar />
        </div>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => dispatch(resetFilter())}
        >
          <RotateCcw data-icon="inline-start" />
          {t("usage.control.reset")}
        </Button>
      </div>

      <div className="dashboard-grid">
        <div className="dashboard-span-4">
          <TokenHero filter={filter} />
        </div>
        <div className="dashboard-span-8">
          <UsageTrendChart filter={filter} />
        </div>
        <div className="dashboard-span-12">
          <KpiBand filter={filter} />
        </div>
      </div>

      <section className="dashboard-section" aria-labelledby="usage-breakdown">
        <div className="dashboard-section-heading">
          <h2 id="usage-breakdown">{t("usage.dashboard.breakdown")}</h2>
          <p>{t("usage.dashboard.breakdownDesc")}</p>
        </div>
        <div className="dashboard-grid">
          <div className="dashboard-span-4">
            <ModelDistribution
              filter={filter}
              onPickModel={(m) => dispatch(patchFilter({ model: m }))}
              onClearModel={() => dispatch(patchFilter({ model: "" }))}
            />
          </div>
          <div className="dashboard-span-8">
            <SessionRanking filter={filter} />
          </div>
          <div
            className={cn(
              multiDevice ? "dashboard-span-7" : "dashboard-span-12",
            )}
          >
            <ProjectSection filter={filter} />
          </div>
          {multiDevice ? (
            <div className="dashboard-span-5">
              <DeviceSection filter={filter} />
            </div>
          ) : null}
        </div>
      </section>

      <section className="dashboard-section" aria-labelledby="usage-patterns">
        <div className="dashboard-section-heading">
          <h2 id="usage-patterns">{t("usage.dashboard.patterns")}</h2>
          <p>{t("usage.dashboard.patternsDesc")}</p>
        </div>
        <div className="dashboard-grid">
          <div className="dashboard-span-12">
            <CalendarHeatmap
              filter={filter}
              spanDays={spanDays}
              onPickDay={(day) =>
                dispatch(patchFilter(dayRangePatch(day, day)))
              }
            />
          </div>
          <div className="dashboard-span-6">
            <DailyCostChart filter={filter} />
          </div>
          <div className="dashboard-span-6">
            <DailyRequestChart filter={filter} />
          </div>
          <div className="dashboard-span-6">
            <TurnDistribution filter={filter} />
          </div>
          <div className="dashboard-span-6">
            <DurationDistribution filter={filter} />
          </div>
        </div>
      </section>

      <RecentRequests />
    </div>
  )
}
