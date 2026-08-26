// ProjectSelect — 项目维度筛选的单一下拉（#102 全站贯通）。
//
// 读 useProjectCandidates（distinct-projects 端点，facet 语义）+ 读写共享
// filterSlice 的 project 维度，选项派生收敛在 derive.ts 的 projectOptions
// （纯函数，测试直断生产路径）。「未知项目」以端点数据过线（unknown 字段），
// 本组件零哨兵字面量。单一「横排」形态：选中「全部」时显全称「全部项目」
// 自带身份；sessions 工具栏经 className 覆盖为无边框本地风格。
//
// project 在全局 filterSlice，故看板 / 日志 / 会话工作台的项目筛选一并跟随；
// 会话左树的项目轨道选中也写同一维度（见 use-sessions-browser）。

import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"
import { FilterSelect } from "@/components/filter-select"

import { projectOptions } from "../derive"
import { useProjectCandidates } from "../use-project-candidates"

export function ProjectSelect({
  /** 覆盖默认样式（sessions 工具栏的无边框本地风格）；默认与 usage 侧
   *  模型/来源下拉一致。 */
  className,
}: {
  className?: string
}) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  const { projects, unknownOption, unknownValue } = useProjectCandidates()
  const options = useMemo(
    () =>
      projectOptions(
        projects,
        unknownOption,
        unknownValue,
        filter.project,
        t("usage.control.unknownProject"),
      ),
    [projects, unknownOption, unknownValue, filter.project, t],
  )
  return (
    <FilterSelect
      ariaLabel={t("usage.control.project")}
      allLabel={t("usage.control.allProject")}
      options={options}
      value={filter.project}
      onChange={(v) => dispatch(patchFilter({ project: v }))}
      className={
        className ??
        // 横排项目路径偏长，宽度内容自适应、上限 w-40，超长由 line-clamp-1
        // 截断（全路径在下拉项内同样截断）。
        "border-border bg-card hover:bg-hover h-8 max-w-40 rounded-md"
      }
      align="start"
    />
  )
}
