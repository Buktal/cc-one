// FilterBar — 五维筛选条的唯一装配：时间 · 来源 · 模型 · 项目 · 设备。名册
// （chip 清单 / 顺序 / 门控）定义在 lib/filter-bar-roster 的 filterBarRoster
// （纯函数，测试直断）；此前 usage 的 ControlBar 与 sessions 工具条各手写一
// 遍同一份装配（架构审查Ⅳ候选⑫收敛），新维度或改门控如今只动一处。
//
// 与各维度控件的关系：全部读写共享 filterSlice——时间经 useDateRangeFilter
//（全仓唯一的组件级适配点，usage/sessions 两份手写适配器就此只剩这一份）、
// 来源 / 模型经 useFacetDimension（候选⑦的 facet 壳）、项目 / 设备直接用
// ProjectSelect / DeviceScopeControl。来源门控（ALL_TIME distinct 非空才显示）
// 也在这里：筛选条渲染结果 = 名册逐位映射成 chip。
//
// 本体只渲染名册 fragment——横排容器由视图定（看板 / 日志 flex-wrap 换行、
// 会话工具条横向滚动且与搜索 / 批量同行），容器是视图级布局而非名册知识；
// 调用方必须把 fragment 放进 flex 容器。
//
// 域间差异注入（仅此三处，其余知识都在本组件）：
//  - sourceLabelOf：来源展示名两域有意不同（usage 短名 "Codex" / sessions
//    全名 "Codex CLI"，见 lib/facet-dimension 头注）。
//  - showDevice：设备位门控（sessions 仅收藏轨；看板 / 日志默认恒显）。
//  - dateTrailingSlot：时间 chip 之后、来源之前的域内 slot（sessions 窄档
//    树容器下拉，@max-[48rem]/sessions 才现身）。
// chip 文案键沿用 usage.control.*（同 DateRangeChip 先例：两域三语逐字相同，
// 收一处单一归属，不再各持一份同文案键）。

import { Fragment, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { useDistinctSourcesQuery } from "@/app/store/api"
import { ALL_TIME_FILTER } from "@/app/store/slices/filterSlice"
import { DateRangeChip } from "@/components/date-range-chip"
import { DeviceScopeControl } from "@/components/device-scope-control"
import { FilterSelect } from "@/components/filter-select"
import { ProjectSelect } from "@/components/project-select"
import { useDateRangeFilter } from "@/hooks/use-date-range-filter"
import { useFacetDimension } from "@/lib/facet-dimension"
import type { FilterBarChipId } from "@/lib/filter-bar-roster"
import { filterBarRoster } from "@/lib/filter-bar-roster"

export interface FilterBarProps {
  /** 来源 tag → 展示名（两域有意不同，见头注）。 */
  sourceLabelOf: (tag: string) => string
  /** 设备位门控：默认 true（看板 / 日志）；会话工作台传 track === "favorites"。 */
  showDevice?: boolean
  /** 时间 chip 之后注入的域内 slot（sessions 窄档树容器下拉）。 */
  dateTrailingSlot?: ReactNode
}

export function FilterBar({
  sourceLabelOf,
  showDevice = true,
  dateTrailingSlot,
}: FilterBarProps) {
  const { t } = useTranslation()
  const range = useDateRangeFilter()
  const sourceFacet = useFacetDimension({
    dimension: "source",
    labelOf: sourceLabelOf,
  })
  const modelFacet = useFacetDimension({ dimension: "model" })
  // 门控探针与候选分属两条查询：门控看 ALL_TIME（任意历史窗口采到过来源就
  // 显示，与用户当前窗口无关），候选看当前窗口的 facet 口径。
  const { data: anySources = [] } = useDistinctSourcesQuery(ALL_TIME_FILTER)

  // 名册 id → chip 元素。Record 键被 FilterBarChipId 收紧：名册加维度时这里
  // 缺元素即编译失败（装配知识与名册定义互锁）。
  const chips: Record<FilterBarChipId, ReactNode> = {
    date: (
      <DateRangeChip
        preset={range.preset}
        fromDay={range.fromDay}
        toDay={range.toDay}
        onPreset={range.onPreset}
        onFromDay={range.onFromDay}
        onToDay={range.onToDay}
        align="start"
      />
    ),
    source: (
      <FilterSelect
        ariaLabel={t("usage.control.source")}
        allLabel={t("usage.control.allSource")}
        options={sourceFacet.options}
        value={sourceFacet.value}
        onChange={sourceFacet.onChange}
        align="start"
      />
    ),
    model: (
      <FilterSelect
        ariaLabel={t("usage.control.model")}
        allLabel={t("usage.control.allModel")}
        options={modelFacet.options}
        value={modelFacet.value}
        onChange={modelFacet.onChange}
        align="start"
      />
    ),
    project: <ProjectSelect />,
    device: <DeviceScopeControl />,
  }

  return (
    <>
      {filterBarRoster({
        hasSources: anySources.length > 0,
        showDevice,
      }).map((id) => (
        <Fragment key={id}>
          {id === "date" ? dateTrailingSlot : null}
          {chips[id]}
        </Fragment>
      ))}
    </>
  )
}
