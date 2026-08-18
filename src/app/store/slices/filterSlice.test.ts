import dayjs from "dayjs"
import { describe, expect, it } from "vitest"

import filterReducer, {
  ALL_TIME_FILTER,
  DEFAULT_FILTER,
  dayPatch,
  type FilterState,
  patchFilter,
  presetPatch,
  toFilter,
} from "@/app/store/slices/filterSlice"
import { dayStr } from "@/lib/date-range"

/** Baseline = "all time" (unconstrained on every dimension). Tests override
 *  fields on top of this. DEFAULT_FILTER itself is "today", so it is NOT the
 *  null baseline. */
const base = (over: Partial<FilterState> = {}): FilterState => ({
  ...ALL_TIME_FILTER,
  ...over,
})

describe("DEFAULT_FILTER (app-start default)", () => {
  it("is the 'today' preset with no other constraints — not persisted", () => {
    expect(DEFAULT_FILTER.range_preset).toBe("today")
    expect(DEFAULT_FILTER.from_day).toBe("")
    expect(DEFAULT_FILTER.to_day).toBe("")
    expect(DEFAULT_FILTER.model).toBe("")
    expect(DEFAULT_FILTER.source).toBe("")
    expect(DEFAULT_FILTER.device_scope).toBe("")
  })
})

describe("toFilter", () => {
  // toFilter is the production path: every usage endpoint's queryFn calls it
  // to turn the Redux FilterState into the UsageFilter the backend expects
  // (architecture.md — tests must cover the real call path).

  it("maps an unconstrained ('all') filter to all-null UsageFilter fields", () => {
    expect(toFilter(base())).toEqual({
      from_ts: null,
      to_ts: null,
      model: null,
      source: null,
      device_scope: null,
    })
  })

  it("passes non-empty model / source / device_scope through", () => {
    const f = toFilter(
      base({
        model: "claude-3-5-sonnet",
        source: "claude_code",
        device_scope: "abc123def456",
      }),
    )
    expect(f.model).toBe("claude-3-5-sonnet")
    expect(f.source).toBe("claude_code")
    expect(f.device_scope).toBe("abc123def456")
  })

  it("converts a custom day range to ISO timestamp bounds ordered from <= to", () => {
    // dayjs formats in the local zone, so assert on ordering, not exact instants.
    const f = toFilter(
      base({
        range_preset: "custom",
        from_day: "2026-07-01",
        to_day: "2026-07-28",
      }),
    )
    expect(f.from_ts).not.toBeNull()
    expect(f.to_ts).not.toBeNull()
    expect(new Date(f.from_ts as string).getTime()).toBeLessThanOrEqual(
      new Date(f.to_ts as string).getTime(),
    )
  })

  it("omits the timestamp bound when the stored day is blank (custom)", () => {
    expect(
      toFilter(base({ range_preset: "custom", to_day: "" })).to_ts,
    ).toBeNull()
    expect(
      toFilter(base({ range_preset: "custom", from_day: "" })).from_ts,
    ).toBeNull()
  })

  it("re-derives a 'today' preset at query time, ignoring any stored dates", () => {
    // Cross-midnight: a dynamic preset stores no concrete date, and
    // even stale from_day/to_day would be ignored — bounds always roll to the
    // current day. This is the invariant that lets the 60s rollover patch go.
    const f = toFilter(
      base({
        range_preset: "today",
        from_day: "1999-01-01",
        to_day: "1999-01-01",
      }),
    )
    const expectedFrom = dayjs(dayStr()).startOf("day").toISOString()
    const expectedTo = dayjs(dayStr()).endOf("day").toISOString()
    expect(f.from_ts).toBe(expectedFrom)
    expect(f.to_ts).toBe(expectedTo)
  })

  it("re-derives a '7d' preset to the last 7 days ending today", () => {
    const f = toFilter(base({ range_preset: "7d" }))
    expect(f.from_ts).toBe(dayjs(dayStr(-6)).startOf("day").toISOString())
    expect(f.to_ts).toBe(dayjs(dayStr()).endOf("day").toISOString())
  })
})

describe("presetPatch / dayPatch (DateRangeChip write semantics, ADR-0008)", () => {
  // Production path: the shared DateRangeChip surfaces (usage ControlCard /
  // ControlBar and the sessions toolbar) dispatch exactly these patches via
  // useDateRangeFilter — the "dynamic presets store no concrete date" rule is
  // a contract here, not a comment in two views.

  it("preset selection stores the preset with no concrete date", () => {
    const next = filterReducer(
      { filter: DEFAULT_FILTER },
      patchFilter(presetPatch("7d")),
    )
    expect(next.filter).toEqual({ ...DEFAULT_FILTER, range_preset: "7d" })
  })

  it("preset selection clears stale stored days", () => {
    const stale: FilterState = {
      ...DEFAULT_FILTER,
      range_preset: "custom",
      from_day: "2026-08-01",
      to_day: "2026-08-02",
    }
    const next = filterReducer(
      { filter: stale },
      patchFilter(presetPatch("today")),
    )
    expect(next.filter).toEqual({ ...DEFAULT_FILTER, range_preset: "today" })
  })

  it("a from-day edit flips to custom and stores the literal day", () => {
    const next = filterReducer(
      { filter: DEFAULT_FILTER },
      patchFilter(dayPatch("from_day", "2026-08-01")),
    )
    expect(next.filter).toEqual({
      ...DEFAULT_FILTER,
      range_preset: "custom",
      from_day: "2026-08-01",
    })
  })

  it("a to-day edit flips to custom independently", () => {
    const next = filterReducer(
      { filter: DEFAULT_FILTER },
      patchFilter(dayPatch("to_day", "2026-08-02")),
    )
    expect(next.filter).toEqual({
      ...DEFAULT_FILTER,
      range_preset: "custom",
      to_day: "2026-08-02",
    })
  })
})
