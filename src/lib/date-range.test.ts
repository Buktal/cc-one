import { describe, expect, it } from "vitest"

import { type DayRange, dayStr, effectiveDays } from "./date-range"

const base = (over: Partial<DayRange> = {}): DayRange => ({
  range_preset: "all",
  from_day: "",
  to_day: "",
  ...over,
})

describe("effectiveDays", () => {
  it("recomputes a dynamic preset instead of trusting frozen stored dates", () => {
    // The bug: a "today" picked yesterday stored yesterday's date; the app
    // left running across midnight must still mean "today".
    const f = base({
      range_preset: "today",
      from_day: "1999-01-01",
      to_day: "1999-01-01",
    })
    expect(effectiveDays(f)).toEqual({ from_day: dayStr(), to_day: dayStr() })
  })

  it("recomputes 7d / 30d bounds ending today", () => {
    const f7 = base({ range_preset: "7d", from_day: "1999-01-01" })
    expect(effectiveDays(f7)).toEqual({
      from_day: dayStr(-6),
      to_day: dayStr(),
    })
    const f30 = base({ range_preset: "30d", to_day: "1999-01-01" })
    expect(effectiveDays(f30)).toEqual({
      from_day: dayStr(-29),
      to_day: dayStr(),
    })
  })

  it("recomputes the 1y preset to a 365-day window ending today", () => {
    const f1y = base({ range_preset: "1y", from_day: "1999-01-01" })
    expect(effectiveDays(f1y)).toEqual({
      from_day: dayStr(-364),
      to_day: dayStr(),
    })
  })

  it("returns stored bounds verbatim for all / custom", () => {
    // "all" stores empty bounds; "custom" keeps the user-picked days.
    expect(effectiveDays(base({ range_preset: "all" }))).toEqual({
      from_day: "",
      to_day: "",
    })
    const c = base({
      range_preset: "custom",
      from_day: "2026-01-15",
      to_day: "2026-01-20",
    })
    expect(effectiveDays(c)).toEqual({
      from_day: "2026-01-15",
      to_day: "2026-01-20",
    })
  })
})
