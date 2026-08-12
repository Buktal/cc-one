import { describe, expect, it } from "vitest"

import { shouldAutoTuck } from "./use-auto-tuck"

// shouldAutoTuck is the gate in front of the auto-tuck timer — the decision
// that an invisible full window should start counting down to the mini bar.
// The timer itself (window events, setTimeout) lives in the hook; this pure
// predicate is the testable part.

describe("shouldAutoTuck", () => {
  it("tucks a minimized full window with a configured delay", () => {
    expect(
      shouldAutoTuck({
        mode: "full",
        delaySecs: 30,
        minimized: true,
        visible: false,
      }),
    ).toBe(true)
  })

  it("tucks a hidden-to-tray full window (invisible but not minimized)", () => {
    expect(
      shouldAutoTuck({
        mode: "full",
        delaySecs: 30,
        minimized: false,
        visible: false,
      }),
    ).toBe(true)
  })

  it("never tucks a visible full window that merely lost focus", () => {
    expect(
      shouldAutoTuck({
        mode: "full",
        delaySecs: 30,
        minimized: false,
        visible: true,
      }),
    ).toBe(false)
  })

  it("never tucks when the delay is 0 (off)", () => {
    expect(
      shouldAutoTuck({
        mode: "full",
        delaySecs: 0,
        minimized: true,
        visible: false,
      }),
    ).toBe(false)
  })

  it("never tucks a lightweight window (already docked)", () => {
    expect(
      shouldAutoTuck({
        mode: "lightweight",
        delaySecs: 30,
        minimized: true,
        visible: false,
      }),
    ).toBe(false)
  })
})
