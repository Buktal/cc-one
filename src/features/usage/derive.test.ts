import dayjs from "dayjs"
import { describe, expect, it } from "vitest"

import {
  hourlySnapshot,
  modelMetricValue,
  stopReasonTone,
  tokenSnapshot,
  topNModels,
  zeroFillTrend,
  zeroTrendPoint,
} from "@/features/usage/derive"

import type {
  ModelStatsRow,
  TrendPoint,
  UsageStats,
} from "@/types/generated/bindings"

function trend(day: string, total: number): TrendPoint {
  return {
    day,
    total_tokens: total,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    total_cost_usd: 0,
  }
}

function stats(totalTokens: number): UsageStats {
  return {
    request_count: 0,
    total_tokens: totalTokens,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    cache_hit_rate: 0,
    total_cost_usd: 0,
    turn_count: 0,
    avg_turn_duration_ms: 0,
  }
}

function modelRow(
  model: string,
  tokens: number,
  cost: number,
  cache_hit_rate: number | null = 0,
): ModelStatsRow {
  return {
    model,
    request_count: 1,
    total_tokens: tokens,
    total_cost_usd: cost,
    cache_hit_rate,
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

describe("tokenSnapshot", () => {
  it("delta = last vs first when both present and start > 0", () => {
    const snap = tokenSnapshot(stats(300), [trend("d1", 100), trend("d2", 200)])
    expect(snap.deltaPct).toBe(1)
    expect(snap.dailyAvg).toBe(150)
  })

  it("delta is null with fewer than two points", () => {
    expect(tokenSnapshot(stats(50), [trend("d1", 50)]).deltaPct).toBeNull()
    expect(tokenSnapshot(stats(0), []).deltaPct).toBeNull()
  })

  it("delta is null when the start point is zero (avoid div-by-zero)", () => {
    expect(
      tokenSnapshot(stats(0), [trend("d1", 0), trend("d2", 50)]).deltaPct,
    ).toBeNull()
  })

  it("daily average is 0 over an empty window", () => {
    expect(tokenSnapshot(stats(0), []).dailyAvg).toBe(0)
  })
})

describe("hourlySnapshot", () => {
  // now 固定为 8-12 14:30 → cutoff = 14 (0..14 共 15 个已过小时)。
  const now = dayjs("2026-08-12T14:30")
  const todayHour = (h: number, v: number) =>
    trend(`2026-08-12T${String(h).padStart(2, "0")}`, v)
  const yesterdayHour = (h: number, v: number) =>
    trend(`2026-08-11T${String(h).padStart(2, "0")}`, v)

  it("delta = today vs yesterday same hours, avg = sum / elapsed hours", () => {
    const today = Array.from({ length: 15 }, (_, h) => todayHour(h, 100))
    const yesterday = Array.from({ length: 15 }, (_, h) =>
      yesterdayHour(h, 120),
    )
    const snap = hourlySnapshot(today, yesterday, now)
    expect(snap.deltaPct).toBeCloseTo((1500 - 1800) / 1800)
    expect(snap.hourlyAvg).toBe(100)
  })

  it("delta is null when yesterday has no data (first day of collection)", () => {
    const today = Array.from({ length: 15 }, (_, h) => todayHour(h, 100))
    expect(hourlySnapshot(today, [], now).deltaPct).toBeNull()
  })

  it("elapsed hours include hour 0 → avg over exactly one hour", () => {
    const snap = hourlySnapshot(
      [todayHour(0, 50)],
      [yesterdayHour(0, 40)],
      dayjs("2026-08-12T00:10"),
    )
    expect(snap.deltaPct).toBe(0.25)
    expect(snap.hourlyAvg).toBe(50)
  })

  it("missing today buckets count as zero, yesterday buckets outside the cutoff are ignored", () => {
    const today = [todayHour(0, 50)] // T01–T03 missing
    const yesterday = [
      yesterdayHour(0, 10),
      yesterdayHour(1, 10),
      yesterdayHour(2, 10),
      yesterdayHour(3, 10),
    ]
    const snap = hourlySnapshot(today, yesterday, dayjs("2026-08-12T03:00"))
    expect(snap.deltaPct).toBe(0.25) // (50 - 40) / 40
    expect(snap.hourlyAvg).toBe(12.5) // 50 / 4
  })
})

describe("topNModels", () => {
  it("keeps the top-N by metric and aggregates the rest", () => {
    const rows = [
      modelRow("a", 10, 1),
      modelRow("b", 30, 3),
      modelRow("c", 20, 2),
      modelRow("d", 5, 0.5),
    ]
    const res = topNModels(rows, "tokens", 2)
    expect(res.top.map((t) => t.model)).toEqual(["b", "c"])
    expect(res.rest).toEqual({ count: 2, sum: 15 })
    expect(res.total).toBe(65)
  })

  it("switches metric to cost", () => {
    const rows = [modelRow("a", 10, 1), modelRow("b", 30, 3)]
    expect(topNModels(rows, "cost", 1).top[0].model).toBe("b")
  })

  it("no remainder when rows <= topN", () => {
    const res = topNModels([modelRow("a", 1, 1)], "tokens", 5)
    expect(res.rest).toEqual({ count: 0, sum: 0 })
  })

  it("total is >= 1 over empty input so callers can divide safely", () => {
    expect(topNModels([], "tokens", 5).total).toBe(1)
  })

  it("carries the backend cache hit rate through to top rows", () => {
    const rows = [modelRow("a", 10, 1, 0.87), modelRow("b", 30, 3, null)]
    const res = topNModels(rows, "tokens", 5)
    expect(res.top.find((t) => t.model === "a")?.cache_hit_rate).toBe(0.87)
    // null (后端无数据) 兜底为 0, 调用方按 rate > 0 条件渲染。
    expect(res.top.find((t) => t.model === "b")?.cache_hit_rate).toBe(0)
  })
})

describe("modelMetricValue", () => {
  it("treats null cost as 0", () => {
    expect(
      modelMetricValue(
        {
          model: "x",
          request_count: 0,
          total_tokens: 0,
          total_cost_usd: null,
          cache_hit_rate: null,
        },
        "cost",
      ),
    ).toBe(0)
  })
})

describe("stopReasonTone", () => {
  it("maps known reasons to tones", () => {
    expect(stopReasonTone("end_turn")).toBe("success")
    expect(stopReasonTone("tool_use")).toBe("tool")
    expect(stopReasonTone("max_tokens")).toBe("warn")
    expect(stopReasonTone("context_window_exceeded")).toBe("warn")
    expect(stopReasonTone("refusal")).toBe("error")
  })

  it("is case-insensitive", () => {
    expect(stopReasonTone("END_TURN")).toBe("success")
  })

  it("returns null for empty / unknown", () => {
    expect(stopReasonTone("")).toBeNull()
    expect(stopReasonTone("something_new")).toBeNull()
  })
})
