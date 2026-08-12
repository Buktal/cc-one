// Pure read-model derivations for the pricing table: client-side search +
// single-column sort, and offset → page stats. The full list is already loaded;
// these keep rendering snappy past a few hundred entries.

import type { PricingEntry } from "@/types/generated/bindings"

export type PricingSortKey = keyof PricingEntry

/**
 * Case-insensitive search over model key + display name, then an optional
 * single-column sort (numeric when both sides are numbers, else localeCompare).
 * The input is never mutated.
 */
export function filterAndSortPricing(
  entries: PricingEntry[],
  search: string,
  sortKey: PricingSortKey | null,
  sortDir: "asc" | "desc",
): PricingEntry[] {
  const q = search.trim().toLowerCase()
  let list = q
    ? entries.filter(
        (e) =>
          e.model_key.toLowerCase().includes(q) ||
          e.display_name.toLowerCase().includes(q),
      )
    : entries
  if (sortKey) {
    list = [...list].sort((a, b) => {
      const av = a[sortKey] ?? 0
      const bv = b[sortKey] ?? 0
      const cmp =
        typeof av === "number" && typeof bv === "number"
          ? av - bv
          : String(av).localeCompare(String(bv))
      return sortDir === "asc" ? cmp : -cmp
    })
  }
  return list
}

/**
 * Parse a price input string into a USD-per-1M number. Empty input means 0
 * (free); anything non-numeric — e.g. a partial `1e` left over from the number
 * input's scientific notation — also degrades to 0, so the editor never stores
 * NaN. Clamping negatives is deliberately left to the input's `min`; this only
 * guards against NaN.
 */
export function parsePriceInput(v: string): number {
  const n = Number(v)
  return Number.isFinite(n) ? n : 0
}

/**
 * Next sort state after clicking column `k`: the same column flips direction, a
 * new column defaults to asc. Pure so the click decision is testable on its own
 * — `usePricingTable` just applies the result and resets the offset.
 */
export function nextSortState(
  sortKey: PricingSortKey | null,
  sortDir: "asc" | "desc",
  k: PricingSortKey,
): { sortKey: PricingSortKey; sortDir: "asc" | "desc" } {
  if (sortKey === k) {
    return { sortKey: k, sortDir: sortDir === "asc" ? "desc" : "asc" }
  }
  return { sortKey: k, sortDir: "asc" }
}
