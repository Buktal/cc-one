// 图表坐标轴的共享小派生（架构审查候选⑨顺带：两份手写刻度公式的归属地）。

/** X 轴刻度间隔（~每 7 根数据一根刻度）：`max(0, ceil(n/7)-1)`。配合 recharts
 *  的 `interval={preserveStartEnd}`，密集桶时首尾两刻度恒保、中间均匀抽稀——
 *  趋势图与请求分布条共用这一份密度决策。 */
export function tickIntervalFor(count: number): number {
  return Math.max(0, Math.ceil(count / 7) - 1)
}
