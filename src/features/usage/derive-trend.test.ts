import dayjs from "dayjs"
import { describe, expect, it } from "vitest"

import {
  shareStackTrend,
  zeroFillTrend,
  zeroTrendPoint,
} from "@/features/usage/derive-trend"

import type { TrendPoint } from "@/types/generated/bindings"

function trend(day: string, total: number): TrendPoint {
  return {
    day,
    total_tokens: total,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    total_cost_usd: 0,
    request_count: 0,
  }
}

/** TrendPoint with explicit four-bucket values (total = their sum). */
function bucketTrend(day: string, buckets: [number, number, number, number]) {
  const [input, output, creation, read] = buckets
  return {
    ...trend(day, input + output + creation + read),
    input_tokens: input,
    output_tokens: output,
    cache_creation_tokens: creation,
    cache_read_tokens: read,
  }
}

describe("zeroFillTrend", () => {
  it("returns input unchanged when empty (caller keeps its empty state)", () => {
    const now = dayjs("2026-07-30T15:30")
    expect(zeroFillTrend([], now, now)).toEqual([])
  })

  it("pads 00:00 → current hour for today, preserving real records", () => {
    const now = dayjs("2026-07-30T15:30")
    const filled = zeroFillTrend([trend("2026-07-30T15", 999)], now, now)
    // 00:00 … 15:00 inclusive = 16 buckets.
    expect(filled).toHaveLength(16)
    expect(filled[0].day).toBe("2026-07-30T00")
    expect(filled[0].total_tokens).toBe(0)
    expect(filled[15].day).toBe("2026-07-30T15")
    expect(filled[15].total_tokens).toBe(999)
  })

  it("fills every gap with a zero point of the right shape", () => {
    const now = dayjs("2026-07-30T02:30")
    const filled = zeroFillTrend([trend("2026-07-30T02", 5)], now, now)
    expect(filled).toHaveLength(3)
    expect(filled[0]).toEqual(zeroTrendPoint("2026-07-30T00"))
  })

  it("pads a past single day across the full 24h axis, not today's date", () => {
    // Selecting a single past day must zero-fill that day (00:00 → 23:00),
    // not the current day — the original bug fed `dayjs()` (today) and the
    // real records never matched, collapsing the chart to a flat zero line.
    const target = dayjs("2026-07-30T15:30")
    const now = dayjs("2026-07-31T10:00")
    const filled = zeroFillTrend([trend("2026-07-30T15", 999)], target, now)
    // 00:00 … 23:00 = 24 buckets.
    expect(filled).toHaveLength(24)
    expect(filled[0].day).toBe("2026-07-30T00")
    expect(filled[0].total_tokens).toBe(0)
    expect(filled[23].day).toBe("2026-07-30T23")
    expect(filled[15].day).toBe("2026-07-30T15")
    expect(filled[15].total_tokens).toBe(999)
  })
})

describe("shareStackTrend", () => {
  it("normalizes the full four-bucket roster to 1 per point", () => {
    const [p] = shareStackTrend([bucketTrend("2026-08-01", [30, 10, 40, 20])])
    expect(p.input_tokens).toBeCloseTo(0.3)
    expect(p.output_tokens).toBeCloseTo(0.1)
    expect(p.cache_creation_tokens).toBeCloseTo(0.4)
    expect(p.cache_read_tokens).toBeCloseTo(0.2)
    expect(
      p.input_tokens +
        p.output_tokens +
        p.cache_creation_tokens +
        p.cache_read_tokens,
    ).toBeCloseTo(1)
  })

  it("keeps the absolute values in the *_abs fields and the cost passthrough", () => {
    const source = {
      ...bucketTrend("2026-08-01", [30, 10, 40, 20]),
      request_count: 42,
    }
    const [p] = shareStackTrend([source])
    expect(p.input_tokens_abs).toBe(30)
    expect(p.output_tokens_abs).toBe(10)
    expect(p.cache_creation_tokens_abs).toBe(40)
    expect(p.cache_read_tokens_abs).toBe(20)
    expect(p.total_tokens).toBe(100)
    expect(p.total_cost_usd).toBe(0)
    // request_count rides through untouched (the share point stays a
    // structural superset of TrendPoint).
    expect(p.request_count).toBe(42)
  })

  it("hides a bucket from the DENOMINATOR, renormalizing the survivors", () => {
    // visible = {input, output}: cache buckets leave the composition (not
    // counted as 0), so the denominator narrows 100 → 40 and the visible
    // pair still sums to 1.
    const [p] = shareStackTrend(
      [bucketTrend("2026-08-01", [30, 10, 40, 20])],
      new Set(["input_tokens", "output_tokens"]),
    )
    expect(p.input_tokens).toBeCloseTo(0.75)
    expect(p.output_tokens).toBeCloseTo(0.25)
    // The visible pair still sums to 1 — the chart renders only visible
    // buckets, so their shares are the whole story.
    expect(p.input_tokens + p.output_tokens).toBeCloseTo(1)
    // abs fields are untouched by visibility — they stay absolute.
    expect(p.cache_creation_tokens_abs).toBe(40)
  })

  it("maps a zero-total point to all-zero ratios instead of dividing by zero", () => {
    const [p] = shareStackTrend([trend("2026-08-01", 0)])
    expect(p.input_tokens).toBe(0)
    expect(p.output_tokens).toBe(0)
    expect(p.cache_creation_tokens).toBe(0)
    expect(p.cache_read_tokens).toBe(0)
  })

  it("narrows to all-zero ratios when every visible bucket is empty", () => {
    // Denominator 0 with a non-empty roster (visible buckets have no data in
    // this point) — same no-divide-by-zero path as the empty point.
    const [p] = shareStackTrend(
      [bucketTrend("2026-08-01", [0, 0, 40, 20])],
      new Set(["input_tokens", "output_tokens"]),
    )
    expect(p.input_tokens).toBe(0)
    expect(p.output_tokens).toBe(0)
  })

  it("passes an empty series through as an empty series", () => {
    expect(shareStackTrend([])).toEqual([])
  })
})
