// Offset → page stats for client-side paginated tables. Generic table math —
// not feature-specific — shared by the pricing table and the usage log table so
// the offset-clamp rule lives in one place. (A divergent copy once omitted the
// clamp and produced an out-of-range page number in the log table when rows
// shrank beneath the current offset.)

export interface PageStats {
  totalPages: number
  page: number
}

/** Offset → page stats. `totalPages` is at least 1 so a single-page control
 *  never disappears, and `page` is clamped into range — important when the row
 *  count shrinks beneath the current offset (e.g. forgetting a device removes
 *  rows) before the offset has reset. */
export function paginate(
  total: number,
  offset: number,
  pageSize: number,
): PageStats {
  const totalPages = Math.max(1, Math.ceil(total / pageSize))
  const page = Math.min(Math.floor(offset / pageSize) + 1, totalPages)
  return { totalPages, page }
}

/** 1-based page → row offset. The single page↔offset conversion — the paged
 *  views used to hand-write `(p - 1) * PAGE_SIZE` at every PaginationBar
 *  wiring site (architecture-sweep candidate ⑧ converges them here). */
export function pageOffset(page: number, pageSize: number): number {
  return Math.max(0, (page - 1) * pageSize)
}

/** Offset clamp target when a result set shrinks: the first offset of the last
 *  page, so deleting the last row of the last page lands on the last page's
 *  first row instead of a page past the end. `total` ≤ 0 → 0. */
export function lastPageStart(total: number, pageSize: number): number {
  return (Math.max(1, Math.ceil(total / pageSize)) - 1) * pageSize
}

/** A page number, or an ellipsis gap marker. */
export type PageNumber = number | "…"

/**
 * Page-number sequence for a pager bar: always 1 and the last page, the
 * current page ±1, with ellipsis gaps in between. 7 or fewer pages render
 * fully. `page` is clamped into range. Pure — the pager bar (request-log /
 * sessions / pricing / library) renders this directly.
 */
export function pageNumbers(page: number, totalPages: number): PageNumber[] {
  const last = Math.max(totalPages, 1)
  const cur = Math.min(Math.max(page, 1), last)
  if (last <= 7) return Array.from({ length: last }, (_, i) => i + 1)
  const around = new Set([1, last, cur - 1, cur, cur + 1])
  const sorted = [...around]
    .filter((p) => p >= 1 && p <= last)
    .sort((a, b) => a - b)
  const out: PageNumber[] = []
  let prev = 0
  for (const p of sorted) {
    if (prev && p - prev > 1) out.push("…")
    out.push(p)
    prev = p
  }
  return out
}
