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
import { paginate } from "@/lib/pagination"

/** Client-side page size — the full list is already loaded; rendering all of
 * it at once jank-scrolls once it grows past a few hundred entries. 20 matches
 * the request-log / sessions / library tables so every paged view uses the
 * same density. */
export const PAGE_SIZE = 20

/**
 * Pricing table controller. Owns the data query, the search/sort/pagination
 * state, and the delete trigger. `setSearch` and `onSort` both reset the offset
 * to page 1 (the previous inline behaviour) so the user always lands on the
 * first page of the freshly filtered/sorted result set.
 */
export function usePricingTable() {
  const { data: entries = [], isLoading } = usePricingQuery()
  const [remove] = useDeletePricingMutation()

  const [search, setSearchState] = useState("")
  const [sortKey, setSortKey] = useState<PricingSortKey | null>(null)
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc")
  const [offset, setOffset] = useState(0)

  const filtered = useMemo(
    () => filterAndSortPricing(entries, search, sortKey, sortDir),
    [entries, search, sortKey, sortDir],
  )

  const total = filtered.length
  const { totalPages, page } = paginate(total, offset, PAGE_SIZE)
  const paged = filtered.slice(offset, offset + PAGE_SIZE)

  // setSearch resets the offset so the user lands on page 1 of the new result
  // set — matches the original inline onChange behaviour.
  function setSearch(value: string) {
    setSearchState(value)
    setOffset(0)
  }

  // onSort applies the pure nextSortState decision (same column flips, a new
  // column defaults to asc) and resets the offset so the chosen column is
  // visible from page 1.
  function onSort(k: PricingSortKey) {
    const next = nextSortState(sortKey, sortDir, k)
    setSortKey(next.sortKey)
    setSortDir(next.sortDir)
    setOffset(0)
  }

  function prevPage() {
    setOffset(Math.max(0, offset - PAGE_SIZE))
  }
  function nextPage() {
    setOffset(offset + PAGE_SIZE)
  }

  return {
    isLoading,
    remove,
    search,
    setSearch,
    sortKey,
    sortDir,
    onSort,
    total,
    page,
    totalPages,
    paged,
    prevPage,
    nextPage,
    hasPrev: offset > 0,
    hasNext: offset + PAGE_SIZE < total,
  }
}
