// ControlBar — the shared horizontal meta-controls for the data views:
// time range · source · model · project · device. Pure filter surfaces (the
// collect / sync action lives in the topbar). One consumer-flavored shape:
// every chip is a "bar" chip (a bare value that widens to 全部模型 / 全部来源 /
// 全部项目 / 全部设备 when "all" is selected, so the chip carries its own
// identity without an external label). The dashboard's filter row (in flow)
// and the logs view's header both render it as-is. Chips 按内容自适应宽度
// (SelectTrigger 默认 w-fit + max-w-* 上限): 「全部」态窄, 长名截断 —— 小窗口
// 一行能多塞几个 chip。 来源 (source) 在 多来源 (sources.length > 0) 时才出现
// —— 采到任意来源就显示, 与设备维度同理。

import { useTranslation } from "react-i18next"
import { useDistinctSourcesQuery } from "@/app/store/api"
import { ALL_TIME_FILTER } from "@/app/store/slices/filterSlice"
import { DateRangeChip as SharedDateRangeChip } from "@/components/date-range-chip"
import { FilterSelect } from "@/components/filter-select"
import { useDateRangeFilter } from "@/hooks/use-date-range-filter"
import { useFacetDimension } from "@/lib/facet-dimension"
import { sourceLabel } from "../source-labels"
import { DeviceScopeControl } from "./device-scope-control"
import { ProjectSelect } from "./project-select"

/** 日期范围 chip —— 把 Redux filterSlice 适配成受控共享组件。数据语义与
 *  sessions 工具栏版一致: 动态预设 (today/7d/30d) 只存 preset、不存具体日期
 *  (日期在 queryFn 实时派生); 日历选日期转 custom 并存具体值. 共享的 JSX /
 *  标签拼装 / 预设清单在 @/components/date-range-chip, slice 读写经
 *  useDateRangeFilter 单一归属 (补丁形状在 filterSlice 的 presetPatch /
 *  dayPatch). */
function DateRangeChip() {
  const range = useDateRangeFilter()
  return (
    <SharedDateRangeChip
      preset={range.preset}
      fromDay={range.fromDay}
      toDay={range.toDay}
      onPreset={range.onPreset}
      onFromDay={range.onFromDay}
      onToDay={range.onToDay}
      align="start"
    />
  )
}

function ModelChip() {
  const { t } = useTranslation()
  // 装配壳（架构审查候选⑦）：facet 窗口 / 并回 / 读写映射收口在共享 hook。
  const facet = useFacetDimension({ dimension: "model" })
  return (
    <FilterSelect
      ariaLabel={t("usage.control.model")}
      allLabel={t("usage.control.allModel")}
      options={facet.options}
      value={facet.value}
      onChange={facet.onChange}
      // 宽度策略（自适应 + max-w-48 上限）收敛在 FilterSelect 本体。
      align="start"
    />
  )
}

/** 来源 (source) 维度筛选 — 与 ModelChip 对称；label 走本域短名映射。 */
function SourceChip() {
  const { t } = useTranslation()
  const facet = useFacetDimension({ dimension: "source", labelOf: sourceLabel })
  return (
    <FilterSelect
      ariaLabel={t("usage.control.source")}
      allLabel={t("usage.control.allSource")}
      options={facet.options}
      value={facet.value}
      onChange={facet.onChange}
      align="start"
    />
  )
}

/** 横向条 — 看板筛选行（in-flow）与日志页顶部共用。Filters only。单一
 *  flex-wrap 行：chips 内容自适应，一行放不下才自然换行 —— 不做按宽度阈值
 *  的强制折行（曾按 @container 60rem 折两行，但侧边栏占宽后主容器永远到
 *  不了 60rem，等于无条件折两行；小窗口换行的真因是 chip 写死宽度，已改
 *  自适应）。 */
export function ControlBar() {
  const { data: sources = [] } = useDistinctSourcesQuery(ALL_TIME_FILTER)
  const hasSources = sources.length > 0
  // shrink-0：满高布局下筛选行是日志卡上方的兄弟 flex 项，不被压缩。
  return (
    <div className="flex shrink-0 flex-wrap items-center gap-2">
      <DateRangeChip />
      {hasSources ? <SourceChip /> : null}
      <ModelChip />
      <ProjectSelect />
      <DeviceScopeControl />
    </div>
  )
}
