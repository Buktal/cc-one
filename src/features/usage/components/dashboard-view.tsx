// Dashboard view (#119 三期改版) — 单页流式看板，无分区导航：卡片自带标题、
// 从上到下即阅读序，吸顶 tabs + scrollspy（原 01-06 分区跳转）随布局重排
// 一并退役（use-section-scroll 无其他使用方，已删）。The shared filter bar
// (time / source / model / project / device + reset) sits in flow at the top
// — the same shape as the logs view's header.
//
// 网格按「读数叙事」分行（#119 四期：卡按语义归组，两组内部相邻不插花）：
//   总量      TokenHero 4 + 趋势 8        —— 总量锚点与主趋势同屏
//   指标      KPI 数字带 12               —— 紧凑一行九格
//   日历      日历热力 12                 —— 独占整行（宽窗周历 53 列要满幅）
//   维度排行  模型分布 4 + 会话排行 8     —— 第一组相邻，组内优先序
//             项目排行 7 + 设备排行 5       模型 > 会话 > 项目 > 设备；
//                                           设备仅多机注册表渲染（单机没有
//                                           排行可读），单机项目独占整行
//   时间分布  每日成本 6 + 每日请求 6     —— 第二组相邻：两张逐日柱
//             轮次分布 6 + 时长分布 6     —— 两张四档半环（原 session/
//                                            request 分区按卡语义拆散归组）
//   明细      近期请求                    —— 页脚流水
// Single frozen layer by design; flat solid bg-card surface (index.css 平面
// 铁律: no glass, no backdrop-blur).

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
    <div className="mx-auto flex w-full max-w-[1380px] flex-col gap-3 pb-4">
      {/* 筛选行 —— 与日志页同形（整宽 in-flow）。重置右贴行尾。 */}
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

      <div className="grid gap-3 min-[1080px]:grid-cols-12">
        <div className="min-[1080px]:col-span-4">
          <TokenHero filter={filter} />
        </div>
        <div className="min-[1080px]:col-span-8">
          <UsageTrendChart filter={filter} />
        </div>
        <div className="min-[1080px]:col-span-12">
          <KpiBand filter={filter} />
        </div>
        <div className="min-[1080px]:col-span-12">
          <CalendarHeatmap
            filter={filter}
            spanDays={spanDays}
            onPickDay={(day) => dispatch(patchFilter(dayRangePatch(day, day)))}
          />
        </div>

        {/* —— 维度排行组：四卡相邻，左→右即优先序（模型 > 会话 > 项目 >
            设备）。设备卡在场时项目收窄 7/12；单机项目独占整行。 —— */}
        <div className="min-[1080px]:col-span-4">
          <ModelDistribution
            filter={filter}
            onPickModel={(m) => dispatch(patchFilter({ model: m }))}
            onClearModel={() => dispatch(patchFilter({ model: "" }))}
          />
        </div>
        <div className="min-[1080px]:col-span-8">
          <SessionRanking filter={filter} />
        </div>
        <div
          className={cn(
            "min-[1080px]:col-span-12",
            multiDevice && "min-[1080px]:col-span-7",
          )}
        >
          <ProjectSection filter={filter} />
        </div>
        {multiDevice ? (
          <div className="min-[1080px]:col-span-5">
            <DeviceSection filter={filter} />
          </div>
        ) : null}

        {/* —— 时间与分布组：四卡相邻（每日成本 > 每日请求 > 轮次 > 时长）—— */}
        <div className="min-[1080px]:col-span-6">
          <DailyCostChart filter={filter} />
        </div>
        <div className="min-[1080px]:col-span-6">
          <DailyRequestChart filter={filter} />
        </div>
        <div className="min-[1080px]:col-span-6">
          <TurnDistribution filter={filter} />
        </div>
        <div className="min-[1080px]:col-span-6">
          <DurationDistribution filter={filter} />
        </div>
      </div>

      <RecentRequests />
    </div>
  )
}
