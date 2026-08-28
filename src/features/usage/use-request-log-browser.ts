// Request-log browser controller —— 分页家族形态对齐（视图 = 纯渲染 +
// use-*-browser hook）：count/logs 两条读数、分页（usePagedBrowser，密度
// 持久化与维度折入都在控制器内）、展开行状态、空态采集动作。此前这份编排
// 内联在 request-log-table.tsx 组件里，是家族中唯一无 controller hook 的
// 成员。
//
// vitest runs in a node-only environment (no DOM — see vitest.config.ts), so
// renderHook is out of scope; the companion test guards that this module
// imports cleanly in node (it pulls the tauri-specta API + RTK Query hooks) —
// the family's standard smoke guard.

import { useState } from "react"
import { useCountQuery, useLogsQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { useCollectAction } from "@/hooks/use-collect-action"
import { usePagedBrowser } from "@/hooks/use-paged-browser"
import { useDeviceOptions } from "@/lib/device-labels"

/** 每页条数密度跨重启记忆，键名沿用 sessions-page-size 的约定。 */
const PAGE_SIZE_KEY = "cc-one:request-log-page-size"

export function useRequestLogBrowser(filter: FilterState) {
  const [expandedId, setExpandedId] = useState<string | null>(null)
  const { data: total = 0 } = useCountQuery(filter)
  // 分页控制器（架构扫描候选⑧）：offset / 翻页单一归属。filter 身份变化 →
  // 回第 1 页并收起展开行（行所在页可能已不存在）——与 offset 重置同一触发
  // 点；换密度同规则（密度在控制器内折进 scope 身份）。
  const browser = usePagedBrowser({
    scope: filter,
    persistKey: PAGE_SIZE_KEY,
    total,
    onScopeReset: () => setExpandedId(null),
  })
  const {
    data: rows = [],
    isLoading,
    isFetching,
    error,
  } = useLogsQuery({
    filter,
    limit: browser.pageSize,
    offset: browser.offset,
  })
  // 空状态 CTA 复用 sidebar 同一份采集动作 (useCollectAction) —— 不再在此
  // 手写 mutation + toast, 避免分叉 (上一份手写副本就漏了数据新鲜度戳记
  // markCollected/markSynced). multiDevice 决定成功 toast 措辞, 与 shell 一致.
  const multiDevice = useDeviceOptions().length > 0
  const { onCollect, collecting } = useCollectAction(multiDevice)

  /** 行展开的开合（一次只展开一行；再点同行收起）。 */
  function toggleRow(uuid: string): void {
    setExpandedId((current) => (current === uuid ? null : uuid))
  }

  return {
    rows,
    isLoading,
    isFetching,
    error,
    total,
    expandedId,
    toggleRow,
    multiDevice,
    onCollect,
    collecting,
    page: browser.page,
    totalPages: browser.totalPages,
    goToPage: browser.goToPage,
    density: browser.density,
  }
}

export type RequestLogBrowser = ReturnType<typeof useRequestLogBrowser>
