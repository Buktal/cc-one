// 项目维度候选面（架构审查Ⅲ候选①）——「项目筛选的候选从哪来、如何变成下拉
// 选项」的唯一归属。曾住 features/usage 内部路径，而项目维度是全局 filterSlice
// 的共享维度，三个域（usage 看板/日志、sessions 工作台、轻量小窗）反向借用，
// 造成 usage⇄sessions 双向循环；搬到 lib 这个中性归属地后各域全部同向依赖
// （判例：device-labels.ts 的设备身份面同病同治）。
//
// 两层各司其职：
//  - useProjectCandidates()：distinct-projects 端点的订阅口（facet 语义）。
//    「未知项目」哨兵以数据过线（#100 决策）——前端永远不持有哨兵字面量，
//    端点把哨兵作为 `unknown` 带回，且拆成两个视图：unknownOption 是 LIVE
//    presence（窗口内出现未知用量才提供该选项），unknownValue 是记忆值（窗口
//    移过后陈旧选中仍可识别、用于打标签与左树映射）。
//  - projectOptions()：候选 → FilterOption 的纯函数（basename 展示 + 哨兵
//    特殊选项 + 已选值并回），测试直断生产路径。
//
// vitest（node 环境）可 import：RTK Query hook 在 hook 体内获取，模块顶层无
// 外部资源句柄。

import { useMemo, useRef } from "react"

import { useDistinctProjectsQuery } from "@/app/store/api"
import { useAppSelector } from "@/app/store/hooks"
import { type FilterOption, facetOptions } from "@/lib/filter-options"
import { projectBasename } from "@/lib/paths"

export function useProjectCandidates(): {
  projects: string[]
  unknownOption: string | null
  unknownValue: string | null
} {
  const filter = useAppSelector((s) => s.filter.filter)
  const facetFilter = useMemo(() => ({ ...filter, project: "" }), [filter])
  const { data } = useDistinctProjectsQuery(facetFilter)
  const lastUnknown = useRef<string | null>(null)
  if (data?.unknown != null) lastUnknown.current = data.unknown
  return {
    projects: data?.projects ?? [],
    unknownOption: data?.unknown ?? null,
    unknownValue: data?.unknown ?? lastUnknown.current,
  }
}

/**
 * Project-dropdown options from the distinct-projects candidates. Known
 * identities show their basename (the tree / table convention — full paths
 * live on hover there and stay out of the dropdown); the unknown sentinel
 * shows the labeled special option. `unknownOption` is the LIVE presence
 * probe (the option is offered only while the endpoint reports unknown usage
 * in the window), while `unknownValue` is the stable value (remembered after
 * first sight) used for LABELING — a selected sentinel that a window change
 * dropped from the candidates still merges back via facetOptions and must
 * still read as「未知项目」, not as its raw literal. A stale known project
 * merges back the same way and keeps its basename label.
 */
export function projectOptions(
  projects: readonly string[],
  unknownOption: string | null,
  unknownValue: string | null,
  selected: string,
  unknownLabel: string,
): FilterOption[] {
  const candidates = unknownOption
    ? facetOptions([...projects, unknownOption], selected)
    : facetOptions(projects, selected)
  return candidates.map((v) => ({
    value: v,
    label:
      unknownValue != null && v === unknownValue
        ? unknownLabel
        : projectBasename(v),
  }))
}
