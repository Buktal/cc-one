import { describe, expect, it } from "vitest"

import { calendarCells } from "@/features/usage/derive-rhythm"

import type { TrendPoint } from "@/types/generated/bindings"

function dayTrend(
  day: string,
  total: number,
  extra: Partial<TrendPoint> = {},
): TrendPoint {
  return {
    day,
    total_tokens: total,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    total_cost_usd: 0,
    request_count: 0,
    ...extra,
  }
}

// 2026-08-10 is a Monday — the ideal grid anchor.
const WEEK = [
  "2026-08-10",
  "2026-08-11",
  "2026-08-12",
  "2026-08-13",
  "2026-08-14",
  "2026-08-15",
  "2026-08-16",
]

describe("calendarCells", () => {
  it("returns empty for an empty window", () => {
    expect(calendarCells([])).toEqual([])
  })

  it("places a Monday-start window in one column: row = i, col = 0", () => {
    const cells = calendarCells(WEEK.map((d, i) => dayTrend(d, i + 1)))
    expect(cells).toHaveLength(7)
    expect(cells[0]).toMatchObject({ day: "2026-08-10", col: 0, row: 0 })
    expect(cells[6]).toMatchObject({ day: "2026-08-16", col: 0, row: 6 })
  })

  it("offsets the first column when the window starts mid-week", () => {
    // 2026-08-12 is a Wednesday → Monday-first row 2, two lead blanks.
    const cells = calendarCells([
      dayTrend("2026-08-12", 5),
      dayTrend("2026-08-13", 5),
    ])
    expect(cells[0]).toMatchObject({ col: 0, row: 2 })
    expect(cells[1]).toMatchObject({ col: 0, row: 3 })
  })

  it("wraps into the next week column after Sunday", () => {
    const cells = calendarCells([
      dayTrend("2026-08-16", 5), // Sunday
      dayTrend("2026-08-17", 5), // next Monday
    ])
    expect(cells[0]).toMatchObject({ col: 0, row: 6 })
    expect(cells[1]).toMatchObject({ col: 1, row: 0 })
  })

  it("levels by quartile of the NON-ZERO days (NONE + four steps)", () => {
    // Non-zero values: 10, 20, 30, 40 → q(p) picks nz[floor(p·4)]:
    // q1=20, q2=30, q3=40; 0 days stay at NONE, 50 tops Q4.
    const values = [0, 10, 20, 30, 40, 50]
    const cells = calendarCells(
      values.map((v, i) => dayTrend(`2026-08-1${i}`, v)),
    )
    expect(cells.map((c) => c.level)).toEqual([0, 1, 1, 2, 3, 4])
  })

  it("maps an all-zero window to every cell NONE (no divide-by-zero)", () => {
    const cells = calendarCells(WEEK.map((d) => dayTrend(d, 0)))
    expect(cells.every((c) => c.level === 0)).toBe(true)
  })

  it("carries requests and cost through per cell", () => {
    const [cell] = calendarCells([
      dayTrend("2026-08-10", 100, { request_count: 7, total_cost_usd: 1.25 }),
    ])
    expect(cell.requests).toBe(7)
    expect(cell.cost).toBeCloseTo(1.25)
    expect(cell.tokens).toBe(100)
  })
})
