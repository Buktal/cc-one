// 五维筛选条（时间 · 来源 · 模型 · 项目 · 设备）的名册纯函数——顺序约定与
// 门控规则的唯一定义处。此前 usage 的 ControlBar 与 sessions 工具条各手写一遍
// 同一份装配（候选⑫收敛前的两份分叉），新维度 / 改门控要同步两处；现在
// @/components/filter-bar 按本函数的输出渲染 chip，本文件是唯一可测的定义点
//（filter-bar-roster.test.ts 直接断言生产路径，与 filter-options.test.ts 同
// 一手法：纯函数层直测组件装配）。
//
// 顺序即产品约定：时间打头（窗口是其余维度的口径前提），来源 / 模型两个
// facet 相邻，项目、设备收尾。

/** 名册上的一颗 chip（id 与 FilterBar 的渲染记录键一一对应）。 */
export type FilterBarChipId = "date" | "source" | "model" | "project" | "device"

export interface FilterBarRosterSpec {
  /** 来源门控：ALL_TIME distinct 非空（任意历史窗口采到过来源）才上名册；
   *  一个来源都没有时来源筛选无从谈起，整颗 chip 消失。 */
  hasSources: boolean
  /** 设备门控：会话工作台仅收藏轨（track === "favorites"）上设备位；看板 /
   *  日志恒 true。单设备的「≤1 台不渲染」在 DeviceScopeControl 内部，不在此。 */
  showDevice: boolean
}

/** 按固定顺序给出当前应渲染的 chip 名册；被门控的维度整颗缺席，其余顺序
 *  不变。 */
export function filterBarRoster(spec: FilterBarRosterSpec): FilterBarChipId[] {
  return [
    "date",
    ...(spec.hasSources ? (["source"] as const) : []),
    "model",
    "project",
    ...(spec.showDevice ? (["device"] as const) : []),
  ]
}
