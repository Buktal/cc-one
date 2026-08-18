// Table state for the pricing view: data (RTK Query) plus client-side search,
// single-column sort and offset pagination, and the delete trigger. This hook
// wires the pure derivations in `derive.ts` (filterAndSortPricing /
// nextSortState) and `lib/pagination.ts` (paginate) to React state so the view
// component stays a thin render. The derivations are reused, not duplicated —
// single source of truth.

import { useMemo, useState } from "react"

import { useDeletePricingMutation, usePricingQuery } from "@/app/store/api"
import {
  filterAndSortPricing,
  nextSortState,
  type PricingSortKey,
} from "@/features/pricing/derive"
import { usePagedBrowser } from "@/hooks/use-paged-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"

/** Client-side page size — the full list is already loaded; rendering all of
 * it at once jank-scrolls once it grows past a few hundred entries. 20 matches
 * the request-log / sessions / library tables so every paged view uses the
 * same density. */
export const PAGE_SIZE = 20

/**
 * Pricing table controller. Owns the data query, the search/sort/pagination
 * state, and the delete trigger. 分页（offset / 页统计 / 翻页 / 删除后夹紧）
 * 走共享控制器 usePagedBrowser（架构扫描候选⑧）：search 与排序变化 → 回第
 * 1 页由 scope 身份变化结构性触发，不再在 setter 里手写 setOffset(0)。
 */
export function usePricingTable() {
  const { data: entries = [], isLoading, error } = usePricingQuery()
  const [removeMut] = useDeletePricingMutation()
  const runWithToast = useMutateWithToast()

  const [search, setSearchState] = useState("")
  const [sortKey, setSortKey] = useState<PricingSortKey | null>(null)
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc")

  const filtered = useMemo(
    () => filterAndSortPricing(entries, search, sortKey, sortDir),
    [entries, search, sortKey, sortDir],
  )

  // 分页控制器（架构扫描候选⑧）：offset / 切片 / 翻页 / 删除后夹紧单一归
  // 属。search / 排序变化 → 回第 1 页（scope 身份变化，结构性规则）；entries
  // （查询数据）不在 scope——数据刷新不重置页，与原先行为一致。
  const browser = usePagedBrowser({
    scope: { search, sortKey, sortDir },
    pageSize: PAGE_SIZE,
    total: filtered.length,
  })
  const total = filtered.length
  const paged = browser.pageItems(filtered)

  // setSearch 不再手写 setOffset(0)：search 是 scope 维度，身份变化由控制器
  // 结构性重置回第 1 页。
  function setSearch(value: string) {
    setSearchState(value)
  }

  // onSort 应用纯函数 nextSortState 的决策（同列翻转、新列默认 asc）；回第
  // 1 页同样由 scope 身份变化触发（sortKey/sortDir 是 scope 维度）。
  function onSort(k: PricingSortKey) {
    const next = nextSortState(sortKey, sortDir, k)
    setSortKey(next.sortKey)
    setSortDir(next.sortDir)
  }

  /** Delete trigger: toasts the outcome and returns success. On success the
   *  offset is clamped back into the now-shorter list — deleting the last row
   *  of the last page would otherwise leave `paged` empty with the header
   *  hanging bare. Busy 不在此管理——确认框的 busy / 关闭时序收敛在
   *  useConfirmDelete（见 pricing-view）。 */
  async function remove(key: string): Promise<boolean> {
    const ok = await runWithToast(removeMut, key, {
      success: { key: "pricing.toast.deleted", vars: { name: key } },
      failed: { key: "pricing.toast.deleteFailed" },
    })
    if (ok) {
      // `filtered` 是删除前的列表；删除后恰少一行——夹到最后一页开头（公式
      // 在控制器的 clamp / lastPageStart，单一测试面）。
      browser.clamp(filtered.length - 1)
    }
    return ok
  }

  return {
    isLoading,
    error,
    remove,
    search,
    setSearch,
    sortKey,
    sortDir,
    onSort,
    total,
    page: browser.page,
    totalPages: browser.totalPages,
    paged,
    goToPage: browser.goToPage,
  }
}
