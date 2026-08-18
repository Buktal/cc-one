// 筛选下拉的共享纯逻辑：哨兵往返映射 + facet 候选派生。
//
// base-ui Select（shadcn）不收空串 value，于是筛选下拉得用 ALL_FILTER 哨兵
// 表示「全部」。此前 8 份手写下拉各自编码同一组规则（哨兵映射 / 反映射 /
// label 解析 / 已选值并回候选 / 空串禁忌）——组件形状的收敛在
// @/components/filter-select，这里收敛其中的纯函数部分，测试直接断言生产路径。

import { ALL_FILTER } from "@/lib/source-tags"

/** 一个筛选项：value 为筛选值，label 为下拉与 trigger 上的显示名。 */
export interface FilterOption {
  value: string
  label: string
}

/** 空串 → 哨兵：base-ui Select 不收空串 value，「全部」在组件内统一以
 *  ALL_FILTER 表示（对外仍保持「空串 = 全部」的调用方域）。 */
export function toSelectValue(value: string): string {
  return value || ALL_FILTER
}

/** 哨兵 → 空串：把 Select 上报的 value 翻译回调用方域（ALL_FILTER = 全部）。 */
export function fromSelectValue(v: string): string {
  return v === ALL_FILTER ? "" : v
}

/** facet 候选派生：把「当前筛选窗口内出现过的候选」与「已选值」并回成一份
 *  有序去重列表。facet 语义下候选只按其它维度收窄（选了模型再切时间窗，新
 *  窗口的候选中可能没有已选模型），已选值必须并回，否则下拉会空、选中会丢。
 *  此前 usage ModelChip / SourceChip 与 sessions modelOptions 各抄一份。 */
export function facetOptions(
  candidates: readonly string[],
  selected: string,
): string[] {
  const set = new Set(candidates)
  if (selected) set.add(selected)
  return [...set].sort()
}
