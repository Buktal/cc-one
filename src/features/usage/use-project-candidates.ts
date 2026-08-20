// 项目候选的共享读取口 —— 全站项目筛选下拉（看板控制卡 / 请求日志横条 / 会话
// 工具栏）与左树的项目身份映射都从这里拿候选。哨兵以数据过线（#100 决策）：
// 前端永远不持有「未知项目」字面量，窗口内出现未知用量时端点把哨兵值作为
// `unknown` 带回。

import { useMemo, useRef } from "react"

import { useDistinctProjectsQuery } from "@/app/store/api"
import { useAppSelector } from "@/app/store/hooks"

/**
 * Project-dropdown candidates under the current filter window. Facet semantics
 * (own dimension dropped — a picked project never shrinks its own candidate
 * list), same as the model / source chips. Two views of the sentinel:
 * - `unknownOption` — LIVE presence: the endpoint reports it only while the
 *   window contains unknown usage; the dropdown offers the special option
 *   exactly then (candidates reflect the selected window, like source/model).
 * - `unknownValue` — the STABLE value, remembered after first sight: a stale
 *   selection (the window shifted past all unknown usage) must still be
 *   recognized for labeling and tree mapping, so the value outlives presence.
 */
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
