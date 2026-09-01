import dayjs from "dayjs"
import { describe, expect, it } from "vitest"

import {
  DEFAULT_FILTER,
  FILTER_DIMENSIONS,
  type FilterState,
} from "@/app/store/slices/filterSlice"
import {
  classifyStopReason,
  costIsNotable,
  deviceSectionStats,
  filterId,
  groupRowsByDay,
  hourlySnapshot,
  modelMetricValue,
  projectRanking,
  requestHeadline,
  sessionSectionStats,
  stopReasonLabelKey,
  stopReasonTone,
  tokenSnapshot,
  topNModels,
  windowDayCount,
} from "@/features/usage/derive"

import type {
  DeviceUsageRow,
  ModelStatsRow,
  ProjectUsageRow,
  SessionUsageRow,
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
    request_count: 0,
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
    p95_turn_duration_ms: null,
    turn_duration_buckets: [0, 0, 0, 0],
  }
}

/** Usage-grain project bucket fixture (backend order: tokens desc). */
function projectRow(
  project: string,
  tokens: number,
  extra: Partial<ProjectUsageRow> = {},
): ProjectUsageRow {
  return {
    project,
    is_unknown: false,
    session_count: 1,
    request_count: 1,
    total_tokens: tokens,
    input_tokens: tokens,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    cache_hit_rate: 0,
    total_cost_usd: 0,
    last_active_at: "2026-08-15T10:00:00.000Z",
    ...extra,
  }
}

/** Usage-grain session bucket fixture. */
function sessionRow(
  sid: string,
  tokens: number,
  extra: Partial<SessionUsageRow> = {},
): SessionUsageRow {
  return {
    session_id: sid,
    device_id: "d",
    title: `title-${sid}`,
    agent_type: "",
    started_at: "2026-08-15T08:00:00.000Z",
    last_active_at: "2026-08-15T10:00:00.000Z",
    turn_count: 1,
    request_count: 1,
    total_tokens: tokens,
    input_tokens: tokens,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    total_cost_usd: 0,
    ...extra,
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
    expect(res.rest).toEqual({ count: 2, sum: 15, requests: 2 })
    expect(res.total).toBe(65)
  })

  it("switches metric to cost", () => {
    const rows = [modelRow("a", 10, 1), modelRow("b", 30, 3)]
    expect(topNModels(rows, "cost", 1).top[0].model).toBe("b")
  })

  it("no remainder when rows <= topN", () => {
    const res = topNModels([modelRow("a", 1, 1)], "tokens", 5)
    expect(res.rest).toEqual({ count: 0, sum: 0, requests: 0 })
  })

  it("carries request_count through to top rows (#119 field completion)", () => {
    const rows = [modelRow("a", 10, 1), modelRow("b", 30, 3)]
    const res = topNModels(rows, "tokens", 5)
    expect(res.top.map((t) => t.request_count)).toEqual([1, 1])
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

describe("classifyStopReason / stopReasonLabelKey", () => {
  it("labels exact values specifically", () => {
    expect(stopReasonLabelKey("end_turn")).toBe("usage.logs.stopReason.endTurn")
    expect(stopReasonLabelKey("max_tokens")).toBe(
      "usage.logs.stopReason.maxTokens",
    )
    expect(stopReasonLabelKey("refusal")).toBe("usage.logs.stopReason.refusal")
  })

  it("labels compound values by their most specific pattern", () => {
    // "context_window_exceeded" contains both patterns; the label follows the
    // first match (context_window) and the tone stays warn.
    expect(classifyStopReason("context_window_exceeded")).toEqual({
      tone: "warn",
      labelKey: "usage.logs.stopReason.contextWindow",
    })
  })

  it("tone and label always agree (single classifier)", () => {
    for (const v of [
      "end_turn",
      "tool_use",
      "max_tokens",
      "refusal",
      "error",
    ]) {
      const { tone, labelKey } = classifyStopReason(v)
      expect(tone).toBe(stopReasonTone(v))
      expect(labelKey).not.toBeNull()
    }
  })

  it("unknown / empty values have neither tone nor label", () => {
    expect(classifyStopReason("")).toEqual({ tone: null, labelKey: null })
    expect(classifyStopReason("something_new")).toEqual({
      tone: null,
      labelKey: null,
    })
  })
})

describe("costIsNotable", () => {
  it("highlights at the threshold and above", () => {
    expect(costIsNotable(1)).toBe(true)
    expect(costIsNotable(1.5)).toBe(true)
    expect(costIsNotable(0.99)).toBe(false)
    expect(costIsNotable(null)).toBe(false)
    expect(costIsNotable(undefined)).toBe(false)
  })
})

describe("groupRowsByDay", () => {
  const row = (timestamp: string) => ({ timestamp })
  // Local-time strings (no tz suffix): grouping follows the LOCAL day, matching
  // the formatTime display — never UTC, so the separator lands where the user
  // sees the date change (a UTC string near midnight would group differently
  // depending on the machine's timezone).

  it("keeps same-day rows in one group", () => {
    expect(
      groupRowsByDay([row("2026-08-12T10:00:00"), row("2026-08-12T11:00:00")]),
    ).toEqual([
      {
        dayKey: "2026-08-12",
        rows: [row("2026-08-12T10:00:00"), row("2026-08-12T11:00:00")],
      },
    ])
  })

  it("starts a new group at each day boundary (time-desc input)", () => {
    const groups = groupRowsByDay([
      row("2026-08-12T10:00:00"),
      row("2026-08-11T23:00:00"),
      row("2026-08-11T09:00:00"),
    ])
    expect(groups.map((g) => g.dayKey)).toEqual(["2026-08-12", "2026-08-11"])
    expect(groups[1].rows).toHaveLength(2)
  })

  it("falls back to the raw date prefix on unparseable timestamps", () => {
    expect(groupRowsByDay([row("garbage"), row("garbage")])[0].dayKey).toBe(
      "garbage",
    )
  })
})

// ------------------------------------------------------- cache keys --------

describe("filterId — cache-key dimension completeness", () => {
  it("the dimension registry covers every FilterState field", () => {
    expect(new Set(FILTER_DIMENSIONS)).toEqual(
      new Set(Object.keys(DEFAULT_FILTER)),
    )
  })

  it("each dimension changes the cache id (no silent cache sharing)", () => {
    for (const dim of FILTER_DIMENSIONS) {
      const other: FilterState = { ...DEFAULT_FILTER, [dim]: "x" }
      expect(
        filterId(other),
        `${dim} must be part of filterId or two filters share one cache entry`,
      ).not.toBe(filterId(DEFAULT_FILTER))
    }
  })
})

describe("projectRanking (#106 sections)", () => {
  const SENTINEL = "__unknown__"
  const rows: ProjectUsageRow[] = [
    projectRow("/proj/alpha", 500),
    projectRow("/proj/beta", 300),
    projectRow("/proj/gamma", 150),
    projectRow("/proj/delta", 40),
    projectRow(SENTINEL, 10, { is_unknown: true }),
  ]

  it("splits top-N known / rest aggregate / unknown, shares over all buckets", () => {
    const r = projectRanking(rows, 2)
    expect(r.top.map((x) => x.project)).toEqual(["/proj/alpha", "/proj/beta"])
    expect(r.rest).toEqual({
      count: 2,
      tokens: 190,
      sessions: 2,
      requests: 2,
      cost: 0,
    })
    expect(r.unknown?.project).toBe(SENTINEL)
    expect(r.knownCount).toBe(4)
    expect(r.totalTokens).toBe(1000)
    // Top-3 known = 500+300+150 over ALL buckets (incl. unknown).
    expect(r.top3Share).toBeCloseTo(0.95)
  })

  it("keeps the unknown bucket out of top even when it outranks a known project", () => {
    const withBigUnknown = [
      ...rows.slice(0, 1),
      projectRow(SENTINEL, 900, { is_unknown: true }),
    ]
    const r = projectRanking(withBigUnknown, 5)
    expect(r.top.map((x) => x.project)).toEqual(["/proj/alpha"])
    expect(r.unknown?.total_tokens).toBe(900)
  })

  it("nulls rest/top3Share when the buckets are too few", () => {
    const r = projectRanking([projectRow("/only", 10)], 5)
    expect(r.rest).toBeNull()
    expect(r.top3Share).toBeNull()
    expect(r.totalTokens).toBe(10)
  })
})

describe("sessionSectionStats (#106 sections)", () => {
  it("aggregates counts / shares / spans / turn buckets / top-N", () => {
    const rows = [
      sessionRow("s1", 600, {
        turn_count: 2,
        last_active_at: "2026-08-15T12:00:00.000Z",
      }),
      sessionRow("s2", 300, {
        turn_count: 10,
        device_id: "d2",
        agent_type: "task",
      }),
      sessionRow("s3", 100, { turn_count: 20 }),
    ]
    const s = sessionSectionStats(rows, 2)
    expect(s.sessions).toBe(3)
    expect(s.subagents).toBe(1)
    expect(s.subagentShare).toBeCloseTo(1 / 3)
    // Longest span = s1 (08:00 → 12:00 = 4h).
    expect(s.longestSpanMs).toBe(4 * 3600_000)
    expect(s.avgTurns).toBeCloseTo(32 / 3)
    // Bands: 1–3 (s1) / 4–8 (none) / 9–16 (s2) / 17+ (s3).
    expect(s.turnBuckets).toEqual([1, 0, 1, 1])
    expect(s.top.map((x) => x.session_id)).toEqual(["s1", "s2"])
    expect(s.top[0].share).toBeCloseTo(0.6)
    // Top rows carry the completion fields (#119): cost / requests / buckets.
    expect(s.top[0].cost).toBe(0)
    expect(s.top[0].requests).toBe(1)
    expect(s.top[0].buckets).toEqual({
      input: 600,
      output: 0,
      cache_creation: 0,
      cache_read: 0,
    })
  })

  it("cost metric ranks priciest first with cost-denominated shares", () => {
    const rows = [
      sessionRow("cheap", 900, { total_cost_usd: 0.5 }),
      sessionRow("pricy", 100, { total_cost_usd: 2.0 }),
    ]
    const s = sessionSectionStats(rows, 5, "cost")
    expect(s.top.map((x) => x.session_id)).toEqual(["pricy", "cheap"])
    expect(s.top[0].share).toBeCloseTo(2.0 / 2.5)
    expect(s.top[0].cost).toBeCloseTo(2.0)
  })

  it("empty input → zeroed aggregates and null ratios/spans", () => {
    const s = sessionSectionStats([], 5)
    expect(s.sessions).toBe(0)
    expect(s.subagentShare).toBeNull()
    expect(s.longestSpanMs).toBeNull()
    expect(s.avgTurns).toBeNull()
    expect(s.turnBuckets).toEqual([0, 0, 0, 0])
    expect(s.top).toEqual([])
  })
})

describe("deviceSectionStats (#107 section)", () => {
  /** Usage-grain device bucket fixture (backend order: tokens desc). */
  function deviceRow(
    device_id: string,
    tokens: number,
    extra: Partial<DeviceUsageRow> = {},
  ): DeviceUsageRow {
    return {
      device_id,
      request_count: 1,
      total_tokens: tokens,
      input_tokens: tokens,
      output_tokens: 0,
      cache_creation_tokens: 0,
      cache_read_tokens: 0,
      cache_hit_rate: 0,
      total_cost_usd: 0,
      last_active_at: "2026-08-15T10:00:00.000Z",
      ...extra,
    }
  }

  it("counts active devices; top share over the section total", () => {
    const s = deviceSectionStats([deviceRow("d1", 700), deviceRow("d2", 300)])
    expect(s.devices).toBe(2)
    expect(s.topShare).toBeCloseTo(0.7)
    expect(s.totalTokens).toBe(1000)
  })

  it("empty input → 0 devices, null share, total floored at 1", () => {
    const s = deviceSectionStats([])
    expect(s.devices).toBe(0)
    expect(s.topShare).toBeNull()
    expect(s.totalTokens).toBe(1)
  })
})

describe("requestHeadline + windowDayCount", () => {
  it("daily average over the window days and the peak bucket", () => {
    const buckets = [
      { day: "2026-08-14", request_count: 100 },
      { day: "2026-08-15", request_count: 400 },
      { day: "2026-08-16", request_count: 200 },
    ]
    expect(requestHeadline(buckets, 4)).toEqual({
      dailyAvg: 175,
      peakCount: 400,
      peakDay: "2026-08-15",
    })
  })

  it("empty counts → nulls (caller renders dashes)", () => {
    expect(requestHeadline([], 7)).toEqual({
      dailyAvg: null,
      peakCount: null,
      peakDay: null,
    })
  })

  it("windowDayCount is inclusive and null on unbounded windows", () => {
    expect(windowDayCount("2026-08-01", "2026-08-30")).toBe(30)
    expect(windowDayCount("2026-08-01", "2026-08-01")).toBe(1)
    expect(windowDayCount("", "2026-08-01")).toBeNull()
    expect(windowDayCount("2026-08-01", "")).toBeNull()
  })
})
