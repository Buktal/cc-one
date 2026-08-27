// Facet 维度筛选下拉的装配壳（架构审查候选⑦）。一条维度流水线曾被手写四遍：
// usage 的 ModelChip / SourceChip、sessions 工具条的来源与模型下拉——每份都编
// 码着同一组步骤（facet 过滤 memo → distinct 端点 → 已选值并回候选 → 映射成
// FilterOption → 读写下拉值）；source 维度的候选口径还分成两派（窗口内
// distinct vs 静态全集常开），无从分辨哪派是有意决策。本壳把口径定死为一派：
//
//   窗口内候选（facet 语义）——过滤掉维度自身（选了某模型不会把自己的候选
//   缩成只剩它），已选值永远并回（切到没用过它的窗口时选中不丢）。
//
// 这样看板 / 日志 / 会话工作台三处的同一维度选项完全一致。项目维度不进此壳：
// 它的候选端点带「未知项目」哨兵的双视图（live presence + 记忆值），装配收口
// 在 lib/project-candidates 的 useProjectCandidates + projectOptions，没有重复。
//
// labelOf 只在 source 维度必填：来源文案两域有意不同（usage 短名 "Codex"、
// sessions 全名 "Codex CLI"——session 行没有额外上下文消歧），不存在全局正
// 确值。hook 合规靠两端点无条件订阅、skip 未用的一支避免多余请求。

import { useMemo } from "react"

import {
  useDistinctModelsQuery,
  useDistinctSourcesQuery,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"
import type { FilterOption } from "@/lib/filter-options"
import { facetOptions } from "@/lib/filter-options"

export interface FacetDimension {
  /** 窗口内候选 ∪ 已选值（label 已就位，可直接喂 FilterSelect）。 */
  options: FilterOption[]
  /** 当前筛选值（空串 = 全部；FilterSelect 在组件内转哨兵）。 */
  value: string
  onChange: (value: string) => void
}

/** model 维度：模型名即展示名。 */
export function useFacetDimension(spec: { dimension: "model" }): FacetDimension
/** source 维度：label 按域注入（两域文案有意不同，见头注）。 */
export function useFacetDimension(spec: {
  dimension: "source"
  labelOf: (tag: string) => string
}): FacetDimension
export function useFacetDimension(
  spec:
    | { dimension: "model" }
    | { dimension: "source"; labelOf: (tag: string) => string },
): FacetDimension {
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  // facet 口径里的唯一变化：本维度自身清空，其余维度照旧收窄窗口。
  const modelWindow = useMemo(() => ({ ...filter, model: "" }), [filter])
  const sourceWindow = useMemo(() => ({ ...filter, source: "" }), [filter])
  const isModel = spec.dimension === "model"
  const labelOf = spec.dimension === "source" ? spec.labelOf : null
  // 两端点无条件订阅（hook 合规），skip 未用的一支避免多余请求。
  const models = useDistinctModelsQuery(modelWindow, { skip: !isModel })
  const sources = useDistinctSourcesQuery(sourceWindow, { skip: isModel })

  // 候选量级小（几十以内）且联动频繁，不再包一层 memo——选项数组直接每次
  // render 现算，省下依赖清单的维护面。
  const candidates = isModel ? (models.data ?? []) : (sources.data ?? [])
  const selected = isModel ? filter.model : filter.source
  return {
    options: facetOptions(candidates, selected).map((value) => ({
      value,
      label: labelOf ? labelOf(value) : value,
    })),
    value: selected,
    onChange: (value: string) =>
      dispatch(patchFilter(isModel ? { model: value } : { source: value })),
  }
}
