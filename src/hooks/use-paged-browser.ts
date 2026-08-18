// Paged-browser controller (architecture-sweep candidate ⑧) — the single home
// for the five pagination variants that used to hand-code the same rules:
//
// - offset state + page stats (paginate) — backend-paged views (sessions /
//   request-log) read `offset` as the query parameter; client-paged views
//   (library / pricing) slice their filtered list with `pageItems`.
// - 1-based page ↔ offset conversion (`goToPage`) and post-delete clamping
//   (`clamp`) — one tested implementation each, no more hand-written
//   `(p - 1) * PAGE_SIZE` at the wiring sites.
// - the "dimension change must return to page 1" invariant as a STRUCTURAL
//   rule: the `scope` argument is the whole dimension set (the same object the
//   caller passes to its query); a scope identity change resets to page 1.
//   Adding a dimension to the scope object participates automatically — the
//   four hand-maintained reset-dependency arrays are gone.
//
// The transition logic is `pagedBrowserNext`, a pure state machine — vitest
// runs in a node-only environment (no renderHook, see vitest.config.ts), so
// the rules are asserted on the exact code the hook runs (architecture.md:
// "测试必须跑生产路径").

import { useEffect, useRef, useState } from "react"
import { lastPageStart, pageOffset, paginate } from "@/lib/pagination"

/** Controller state: the row offset plus the last-synced scope identity
 *  (serialized). `scopeKey: null` = never synced (mount). */
export interface PagedBrowserState {
  offset: number
  scopeKey: string | null
}

export type PagedBrowserAction =
  | { kind: "scope-sync"; scopeKey: string }
  | { kind: "go-to-page"; page: number }
  | { kind: "shift-pages"; delta: number }
  | { kind: "clamp"; total: number }

/** Scope identity — the serialized dimension set. A dimension change changes
 *  the key; the reset rule compares keys, never hand-listed values. Requires a
 *  JSON-serializable scope (the same data already rides through RTK Query's
 *  serialized cache keys). */
export function scopeKeyOf(scope: unknown): string {
  return JSON.stringify(scope)
}

/** Pure transition function — the controller's single test surface. One branch
 *  per rule: scope-change reset / page↔offset / page shift / post-delete
 *  clamp. */
export function pagedBrowserNext(
  state: PagedBrowserState,
  action: PagedBrowserAction,
  pageSize: number,
): PagedBrowserState {
  switch (action.kind) {
    case "scope-sync":
      // 结构性规则：scope 身份变化 → 回第 1 页；身份不变 → 原样返回（React
      // 借此跳过重渲染）。
      if (action.scopeKey === state.scopeKey) return state
      return { offset: 0, scopeKey: action.scopeKey }
    case "go-to-page":
      return { ...state, offset: pageOffset(action.page, pageSize) }
    case "shift-pages":
      // 行级相邻导航（detail-sheet 的 prev/next 跨页）按页平移 offset；负向
      // 夹到 0，保持页对齐不变量。
      return {
        ...state,
        offset: Math.max(0, state.offset + action.delta * pageSize),
      }
    case "clamp":
      // 删除后夹紧：总行数缩小 → 压回最后一页开头，不留空页。
      return {
        ...state,
        offset: Math.min(state.offset, lastPageStart(action.total, pageSize)),
      }
  }
}

export interface PagedBrowserOptions {
  /** 维度集合（筛选 / 搜索 / 导航…，通常就是查询参数对象）。身份变化 → 回
   *  第 1 页：结构性规则，往 scope 里新增维度自动参与，不需要手列依赖清单。
   *  需可 JSON 序列化（RTK Query 缓存键本来就是序列化参数）。 */
  scope: unknown
  pageSize: number
  /** 过滤后的总行数（后端 count / 客户端 filtered.length）。 */
  total: number
  /** scope 变化触发回第 1 页时的回调，与 offset 重置同一触发点（如
   *  request-log 同时收起展开行——行所在页可能已不存在）。 */
  onScopeReset?: () => void
}

export interface PagedBrowser {
  /** 当前页行偏移——后端分页变体直接作查询参数。 */
  offset: number
  /** 1-based 当前页（paginate 夹紧后）。 */
  page: number
  totalPages: number
  /** 翻到 1-based 页。 */
  goToPage: (page: number) => void
  /** 按页平移（detail-sheet 跨页 prev/next，±1）。 */
  shiftPages: (delta: number) => void
  /** 删除后夹紧：把当前页压回最后一页开头。传缩小后的新总数。 */
  clamp: (total: number) => void
  /** 客户端分页变体：把本地列表切到当前页。 */
  pageItems: <T>(items: readonly T[]) => T[]
}

export function usePagedBrowser({
  scope,
  pageSize,
  total,
  onScopeReset,
}: PagedBrowserOptions): PagedBrowser {
  const [state, setState] = useState<PagedBrowserState>({
    offset: 0,
    scopeKey: null,
  })
  const scopeKey = scopeKeyOf(scope)

  // 结构性重置：scope 身份变化 → 回第 1 页。ref 只作「上次同步身份」的记忆
  // ——同步更新，StrictMode 双跑 effect 不会重复触发；判定与迁移都走
  // pagedBrowserNext（生产路径，测试直接覆盖）。首帧身份即 ref 初值，挂载
  // 不触发重置（offset 本来就是 0）。
  const prevScopeKey = useRef(scopeKey)
  useEffect(() => {
    if (prevScopeKey.current === scopeKey) return
    prevScopeKey.current = scopeKey
    onScopeReset?.()
    setState((s) =>
      pagedBrowserNext(s, { kind: "scope-sync", scopeKey }, pageSize),
    )
  }, [scopeKey, pageSize, onScopeReset])

  const { offset } = state
  const { totalPages, page } = paginate(total, offset, pageSize)

  function goToPage(p: number): void {
    setState((s) =>
      pagedBrowserNext(s, { kind: "go-to-page", page: p }, pageSize),
    )
  }

  function shiftPages(delta: number): void {
    setState((s) =>
      pagedBrowserNext(s, { kind: "shift-pages", delta }, pageSize),
    )
  }

  function clamp(newTotal: number): void {
    setState((s) =>
      pagedBrowserNext(s, { kind: "clamp", total: newTotal }, pageSize),
    )
  }

  function pageItems<T>(items: readonly T[]): T[] {
    return items.slice(offset, offset + pageSize)
  }

  return { offset, page, totalPages, goToPage, shiftPages, clamp, pageItems }
}
