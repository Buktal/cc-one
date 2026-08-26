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

import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  useDistinctModelsQuery,
  useDistinctSourcesQuery,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { ALL_TIME_FILTER, patchFilter } from "@/app/store/slices/filterSlice"
import { DateRangeChip as SharedDateRangeChip } from "@/components/date-range-chip"
import { FilterSelect } from "@/components/filter-select"
import { useDateRangeFilter } from "@/hooks/use-date-range-filter"
import { facetOptions } from "@/lib/filter-options"
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
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  // Facet filter = 看板筛选去掉 model 维度本身。模型下拉只列「所选时间 / 来源
  // / 设备窗口内真正出现过的模型」, 不按 model 自身收窄 (否则选了 glm 下拉就
  // 只剩 glm); 当前已选模型并回候选, 避免切到没用过它的窗口时 chip 变成空值。
  // 候选跨天滚动靠采集间隔 → usage_changed → invalidate: 动态预设的
  // filter 一天内引用稳定, 无需 dayStr() 触发器。
  const facetFilter = useMemo(() => ({ ...filter, model: "" }), [filter])
  const { data: models = [] } = useDistinctModelsQuery(facetFilter)
  // 并回规则（已选模型并入候选，窗口切换后下拉不空）收敛在 facetOptions。
  const options = useMemo(
    () =>
      facetOptions(models, filter.model).map((m) => ({ value: m, label: m })),
    [models, filter.model],
  )
  return (
    <FilterSelect
      ariaLabel={t("usage.control.model")}
      allLabel={t("usage.control.allModel")}
      options={options}
      value={filter.model}
      onChange={(v) => dispatch(patchFilter({ model: v }))}
      // 模型名最长且不可控 → 横排给最宽上限（内容自适应，超长截断）。
      className="border-border bg-card hover:bg-hover h-8 max-w-48 rounded-md"
      align="start"
    />
  )
}

/** 来源 (source) 维度筛选 — 与 ModelChip 对称, 选项来自 queryDistinctSources. */
function SourceChip() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  // 与 ModelChip 对称: facet 去掉 source 自身, 候选只含所选窗口内出现过的来源;
  // 已选来源并回候选。跨天滚动靠采集间隔刷新, 见 ModelChip。
  const facetFilter = useMemo(() => ({ ...filter, source: "" }), [filter])
  const { data: sources = [] } = useDistinctSourcesQuery(facetFilter)
  // 并回规则（已选来源并入候选，窗口切换后下拉不空）收敛在 facetOptions。
  const options = useMemo(
    () =>
      facetOptions(sources, filter.source).map((s) => ({
        value: s,
        label: sourceLabel(s),
      })),
    [sources, filter.source],
  )
  return (
    <FilterSelect
      ariaLabel={t("usage.control.source")}
      allLabel={t("usage.control.allSource")}
      options={options}
      value={filter.source}
      onChange={(v) => dispatch(patchFilter({ source: v }))}
      className="border-border bg-card hover:bg-hover h-8 max-w-30 rounded-md"
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
  return (
    <div className="flex flex-wrap items-center gap-2">
      <DateRangeChip />
      {hasSources ? <SourceChip /> : null}
      <ModelChip />
      <ProjectSelect />
      <DeviceScopeControl />
    </div>
  )
}
