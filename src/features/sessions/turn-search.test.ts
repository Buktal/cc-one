// Tests for the turn-nav search state machine (architecture.md: "测试必须跑生产
// 路径"). The "when does in-session search exit / when is the hit highlight
// kept" decisions used to live inside TurnNavPanel as four inline handlers,
// the semantics only in JSX comments — untested, so a future edit could change
// an exit path without anyone noticing. They are now a pure reducer; these
// cases lock its transition table in vitest's node-only env.

import { describe, expect, it } from "vitest"

import { initialTurnSearch, reduceTurnSearch } from "./turn-search"

/** The clean state every exit path must land on. */
const CLEAN = { searching: false, query: "", lastJumped: null }

describe("reduceTurnSearch — transition table", () => {
  it("toggle enters search mode clean from the turn index", () => {
    expect(reduceTurnSearch(initialTurnSearch, { type: "toggle" })).toEqual({
      searching: true,
      query: "",
      lastJumped: null,
    })
  })

  it("toggle while searching exits to the clean state", () => {
    const inSearch = {
      searching: true,
      query: "bug",
      lastJumped: "u7",
    }
    expect(reduceTurnSearch(inSearch, { type: "toggle" })).toEqual(CLEAN)
  })

  it("query change keeps search mode, updates the query, drops the highlight", () => {
    const inSearch = {
      searching: true,
      query: "bug",
      lastJumped: "u7",
    }
    expect(reduceTurnSearch(inSearch, { type: "query", query: "fix" })).toEqual(
      { searching: true, query: "fix", lastJumped: null },
    )
  })

  it("hit keeps the clicked uuid highlighted, query untouched", () => {
    const inSearch = { searching: true, query: "bug", lastJumped: null }
    expect(reduceTurnSearch(inSearch, { type: "hit", uuid: "u7" })).toEqual({
      searching: true,
      query: "bug",
      lastJumped: "u7",
    })
  })

  it("esc exits to the clean state", () => {
    const inSearch = {
      searching: true,
      query: "bug",
      lastJumped: "u7",
    }
    expect(reduceTurnSearch(inSearch, { type: "esc" })).toEqual(CLEAN)
  })

  it("clear exits to the clean state", () => {
    const inSearch = {
      searching: true,
      query: "bug",
      lastJumped: "u7",
    }
    expect(reduceTurnSearch(inSearch, { type: "clear" })).toEqual(CLEAN)
  })
})

describe("reduceTurnSearch — the four exit paths all land on the same state", () => {
  it("toggle-close / esc / clear / query-change drop the highlight", () => {
    // The four inline handlers that used to live in TurnNavPanel: the toolbar
    // toggle while searching, Esc, the clear button, and a query change. The
    // first three exit search mode entirely; the fourth stays in search but
    // releases the hit highlight — all four must leave the panel consistent.
    const inSearch = {
      searching: true,
      query: "bug",
      lastJumped: "u7",
    }
    const exits = [
      reduceTurnSearch(inSearch, { type: "toggle" }),
      reduceTurnSearch(inSearch, { type: "esc" }),
      reduceTurnSearch(inSearch, { type: "clear" }),
    ]
    for (const s of exits) expect(s).toEqual(CLEAN)
    expect(
      reduceTurnSearch(inSearch, { type: "query", query: "nope" }).lastJumped,
    ).toBeNull()
  })
})

describe("reduceTurnSearch — full lifecycle", () => {
  it("type → hit → retype → esc: highlight follows the query, exit resets everything", () => {
    let s = initialTurnSearch
    s = reduceTurnSearch(s, { type: "toggle" }) // open search
    expect(s.searching).toBe(true)
    s = reduceTurnSearch(s, { type: "query", query: "bug" })
    s = reduceTurnSearch(s, { type: "hit", uuid: "u7" })
    expect(s.lastJumped).toBe("u7")
    // The eye's anchor lives for the current query only — retyping drops it.
    s = reduceTurnSearch(s, { type: "query", query: "bug fix" })
    expect(s.lastJumped).toBeNull()
    expect(s.query).toBe("bug fix")
    // Esc exits to a clean panel: next open starts fresh, no leftover query.
    s = reduceTurnSearch(s, { type: "esc" })
    expect(s).toEqual(CLEAN)
    s = reduceTurnSearch(s, { type: "toggle" })
    expect(s).toEqual({ searching: true, query: "", lastJumped: null })
  })

  it("a hit on an unknown uuid still highlights it (render simply finds no row)", () => {
    // The panel guards the rowIndex lookup before dispatching; a stale uuid
    // that slips through leaves the panel consistent, not wedged.
    const inSearch = { searching: true, query: "bug", lastJumped: null }
    expect(
      reduceTurnSearch(inSearch, { type: "hit", uuid: "ghost" }).lastJumped,
    ).toBe("ghost")
  })
})
