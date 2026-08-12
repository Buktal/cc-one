// Dashboard view — token-first LLM Usage Cockpit.
//
// 三栏布局: 左导航(Shell) · 中图表(趋势 / 模型分布 / 近期请求) ·
// 右数值(控制卡 / 总消耗锚点 / 概览)。筛选与采集从旧顶部 UsageToolbar +
// CommandBar 收敛进右栏 ControlCard；时间/模型/设备维度都进控制卡，中栏
// 顶部不再空旷。device_scope 也在此统一控制 (单设备时控制卡自动隐藏)。

import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"

import { ControlCard } from "./control-card"
import { KpiStrip } from "./kpi-strip"
import { ModelDistribution } from "./model-distribution"
import { RecentRequests } from "./recent-requests"
import { TokenHero } from "./token-hero"
import { UsageTrendChart } from "./usage-trend-chart"

export function DashboardView() {
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)

  return (
    <div className="flex flex-col gap-4">
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
        {/* 中栏 · 可视化 */}
        <div className="flex flex-col gap-4">
          <UsageTrendChart filter={filter} />
          <ModelDistribution
            filter={filter}
            onPickModel={(m) => dispatch(patchFilter({ model: m }))}
            onClearModel={() => dispatch(patchFilter({ model: "" }))}
          />
          <RecentRequests />
        </div>
        {/* 右栏 · 数值 */}
        <aside className="flex flex-col gap-4">
          <ControlCard />
          <TokenHero filter={filter} />
          <KpiStrip filter={filter} />
        </aside>
      </div>
    </div>
  )
}
