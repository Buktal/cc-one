// Tests for the turn-nav routing reducer (architecture.md: "测试必须跑生产路径").
// The "which user turn is active" decision used to live inside the useTurnNav
// hook as imperative state + a skip flag + two timers — untested, which is how
// the "click turn 14, highlight turn 13" regression went unnoticed. It is now a
// pure reducer; these cases lock its invariant in vitest's node-only env.

import { describe, expect, it } from "vitest"

import type { SessionMessage } from "@/types/generated/bindings"
import {
  initialTurnNav,
  reduceTurnNav,
  turnAnchors,
  turnAtAnchor,
} from "./turn-nav"

/** Build a transcript of `nTurns` user/assistant pairs: u0,a0,u1,a1,… User turn
 *  `i` (0-based) sits at message index `2i`. Mirrors a real Claude transcript's
 *  alternation closely enough to exercise the index math. */
function transcript(nTurns: number): SessionMessage[] {
  const out: SessionMessage[] = []
  for (let i = 0; i < nTurns; i++) {
    out.push({
      uuid: `u${i}`,
      session_id: "s",
      role: "user",
      ts: "",
      content: `user ${i}`,
    })
    out.push({
      uuid: `a${i}`,
      session_id: "s",
      role: "assistant",
      ts: "",
      model: "m",
      content: `assistant ${i}`,
    })
  }
  return out
}

const anchors = (n: number) => turnAnchors(transcript(n))

describe("turnAnchors / turnAtAnchor", () => {
  it("collects user messages in order with their indices", () => {
    expect(anchors(3)).toEqual([
      { index: 0, uuid: "u0" },
      { index: 2, uuid: "u1" },
      { index: 4, uuid: "u2" },
    ])
  })

  it("active = last user turn at or above the scroll anchor", () => {
    const t = anchors(14)
    expect(turnAtAnchor(t, 0)).toBe("u0")
    expect(turnAtAnchor(t, 5)).toBe("u2") // between u2@4 and u3@6
    expect(turnAtAnchor(t, 26)).toBe("u13")
    expect(turnAtAnchor(t, 99)).toBe("u13")
    expect(turnAtAnchor(t, -1)).toBe(null) // above the first turn
  })
})

describe("reduceTurnNav — natural scroll (no jump)", () => {
  it("tracks the turn at the viewport top", () => {
    const t = anchors(14)
    let s = initialTurnNav
    s = reduceTurnNav(s, { type: "range", startIndex: 5 }, t)
    expect(s.activeUuid).toBe("u2")
    s = reduceTurnNav(s, { type: "range", startIndex: 20 }, t)
    expect(s.activeUuid).toBe("u10")
    // re-reporting the same anchor is a no-op (no spurious change)
    s = reduceTurnNav(s, { type: "range", startIndex: 20 }, t)
    expect(s.activeUuid).toBe("u10")
  })
})

describe("reduceTurnNav — jump pinning (the regression)", () => {
  it("holds the jumped turn through a burst of post-jump rangeChanged events", () => {
    const t = anchors(14)
    // User was reading turn 5 (u4@8), clicks turn 14 (u13@26).
    let s = initialTurnNav
    s = reduceTurnNav(s, { type: "range", startIndex: 9 }, t)
    expect(s.activeUuid).toBe("u4")
    // Jump to turn 14 (message index 26).
    s = reduceTurnNav(s, { type: "jump", targetIndex: 26 }, t)
    expect(s.activeUuid).toBe("u13") // turn 14 (0-based u13)
    // Virtuoso fires a BURST of rangeChanged as the long scroll settles (scroll
    // steps + dynamic-height re-measurement). The -72px offset parks turn 14
    // below the viewport top, so every event reports turn 13's content at the
    // top (startIndex in turn 13's span: a12@25). The pin must hold turn 14.
    s = reduceTurnNav(s, { type: "range", startIndex: 25 }, t)
    expect(s.activeUuid).toBe("u13") // ← was u12 (turn 13) under the old skip logic
    s = reduceTurnNav(s, { type: "range", startIndex: 25 }, t) // 2nd burst event
    expect(s.activeUuid).toBe("u13")
    s = reduceTurnNav(s, { type: "range", startIndex: 25 }, t) // 3rd burst event
    expect(s.activeUuid).toBe("u13")
  })
})

describe("reduceTurnNav — pin release", () => {
  it("releases forward when the user scrolls past the next turn", () => {
    const t = anchors(15) // u13@26 is the pin; u14@28 is the turn after it
    let s = reduceTurnNav(initialTurnNav, { type: "jump", targetIndex: 26 }, t) // u13
    s = reduceTurnNav(s, { type: "range", startIndex: 25 }, t) // burst → hold
    expect(s.activeUuid).toBe("u13")
    s = reduceTurnNav(s, { type: "range", startIndex: 28 }, t) // u14@28 → next turn
    expect(s.activeUuid).toBe("u14")
    // pin released → natural tracking resumes
    s = reduceTurnNav(s, { type: "range", startIndex: 24 }, t)
    expect(s.activeUuid).toBe("u12")
  })

  it("releases backward when the user scrolls past the turn before the pin", () => {
    const t = anchors(14)
    let s = reduceTurnNav(initialTurnNav, { type: "jump", targetIndex: 26 }, t) // u13
    s = reduceTurnNav(s, { type: "range", startIndex: 25 }, t) // hold u13
    // prev turn is u12@24; scrolling into u11@22 is a genuine retreat
    s = reduceTurnNav(s, { type: "range", startIndex: 22 }, t)
    expect(s.activeUuid).toBe("u11")
  })

  it("holds the pin on a scroll that stays within the previous turn's span", () => {
    const t = anchors(14)
    let s = reduceTurnNav(initialTurnNav, { type: "jump", targetIndex: 26 }, t) // u13
    // startIndex still inside turn 13 (u12@24 .. a12@25) → hold turn 14
    s = reduceTurnNav(s, { type: "range", startIndex: 24 }, t)
    expect(s.activeUuid).toBe("u13")
  })

  it("a new jump re-pins onto its target", () => {
    const t = anchors(14)
    let s = reduceTurnNav(initialTurnNav, { type: "jump", targetIndex: 26 }, t) // u13
    s = reduceTurnNav(s, { type: "jump", targetIndex: 10 }, t) // u5@10
    expect(s.activeUuid).toBe("u5")
    s = reduceTurnNav(s, { type: "range", startIndex: 9 }, t) // burst → hold
    expect(s.activeUuid).toBe("u5")
  })

  it("jumping onto the first turn holds with no previous turn to release into", () => {
    const t = anchors(14)
    let s = reduceTurnNav(initialTurnNav, { type: "jump", targetIndex: 0 }, t) // u0
    s = reduceTurnNav(s, { type: "range", startIndex: 0 }, t)
    expect(s.activeUuid).toBe("u0")
    // forward past the next turn still releases
    s = reduceTurnNav(s, { type: "range", startIndex: 4 }, t) // u2@4
    expect(s.activeUuid).toBe("u2")
  })
})

describe("reduceTurnNav — jump onto a non-turn row (search hit)", () => {
  it("pins the enclosing user turn", () => {
    const t = anchors(14)
    // search hit on a12@25 (assistant) → enclosing turn is u12 (turn 13)
    let s = reduceTurnNav(initialTurnNav, { type: "jump", targetIndex: 25 }, t)
    expect(s.activeUuid).toBe("u12")
    s = reduceTurnNav(s, { type: "range", startIndex: 24 }, t) // burst → hold u12
    expect(s.activeUuid).toBe("u12")
  })
})
