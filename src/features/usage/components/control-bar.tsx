// ControlBar — usage 域的筛选条绑定：共享 FilterBar 名册（时间 · 来源 · 模型
// · 项目 · 设备——清单 / 顺序 / 门控 / 日期适配器的装配知识单源在
// @/components/filter-bar）+ 本域的两点差异：来源展示名走短名映射
//（sourceLabel，与 sessions 全名有意分叉）、横排容器用 flex-wrap 换行。
// 看板筛选行（in-flow）与日志页顶部共用本组件。chips 按内容自适应宽度
//（FilterSelect / DateRangeChip 本体策略），「全部」态窄、长名截断——小窗口
// 一行能多塞几个 chip，放不下才自然换行（不做按宽度阈值的强制折行：曾按
// @container 60rem 折两行，但侧边栏占宽后主容器永远到不了 60rem，等于无条件
// 折两行，已改自适应）。

import { FilterBar } from "@/components/filter-bar"
import { sourceLabel } from "../source-labels"

/** 横向条 — 看板筛选行（in-flow）与日志页顶部共用。Filters only（采集动作
 *  在 topbar）。shrink-0：满高布局下筛选行是日志卡上方的兄弟 flex 项，不被
 *  压缩。 */
export function ControlBar() {
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2">
      <FilterBar sourceLabelOf={sourceLabel} />
    </div>
  )
}
