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
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
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
  const { data: entries = [], isLoading, error } = usePricingQuery()
  const [removeMut] = useDeletePricingMutation()
  const runWithToast = useMutateWithToast()

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

  /** 1-based page jump for the shared PaginationBar. */
  function goToPage(p: number) {
    setOffset(Math.max(0, (p - 1) * PAGE_SIZE))
  }

  const [removing, setRemoving] = useState(false)

  /** Delete trigger: toasts the outcome and exposes a busy flag for the confirm
   *  dialog (pattern mirrors sessions' deleteGroup). On success the offset is
   *  clamped back into the now-shorter list — deleting the last row of the last
   *  page would otherwise leave `paged` empty with the header hanging bare.
   *
   *  Busy is reset on both success and failure: the dialog closes via prop
   *  (setDeleting(null)), which never fires the view's onOpenChange (Radix only
   *  calls it on user interaction) — leaving busy true would make the next
   *  open spin forever. A one-frame flash-back during the close animation is
   *  harmless. */
  async function remove(key: string): Promise<boolean> {
    setRemoving(true)
    const ok = await runWithToast(removeMut, key, {
      success: { key: "pricing.toast.deleted", vars: { name: key } },
      failed: { key: "pricing.toast.deleteFailed" },
    })
    setRemoving(false)
    if (ok) {
      // `filtered` here is the pre-delete list; the post-delete list is
      // exactly one row shorter, so clamp to the last page's start offset.
      const remaining = filtered.length - 1
      setOffset((o) =>
        Math.min(
          o,
          Math.max(0, Math.floor((remaining - 1) / PAGE_SIZE) * PAGE_SIZE),
        ),
      )
    }
    return ok
  }

  return {
    isLoading,
    error,
    remove,
    removing,
    search,
    setSearch,
    sortKey,
    sortDir,
    onSort,
    total,
    page,
    totalPages,
    paged,
    goToPage,
  }
}
