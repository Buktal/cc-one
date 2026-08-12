// Invariant test for the Store-tag single source of truth.
//
// vitest runs node-only (vitest.config.ts — no DOM), so we cannot exercise
// RTK Query's invalidate behaviour (no test store / no invoke mock). Instead
// we assert on the DECLARED classification in tags.ts and verify it is
// complete against the live api's endpoint names. Adding a read endpoint
// forces a classification decision — the test stays red until the name lands
// in one of the two read sets.

import { describe, expect, it } from "vitest"

import { vaultApi } from "./api"
import {
  INVALIDATE_STORE,
  NON_STORE_READS,
  STORE_DERIVED_READS,
  STORE_TAG,
  storeRead,
  WHOLE_STORE_WRITES,
} from "./tags"

describe("storeRead helper", () => {
  it("always prepends the Store tag", () => {
    expect(storeRead()).toEqual(["Store"])
    expect(storeRead("Devices")).toEqual(["Store", "Devices"])
    expect(storeRead({ type: "Usage", id: "x" })).toEqual([
      "Store",
      { type: "Usage", id: "x" },
    ])
  })
})

describe("INVALIDATE_STORE", () => {
  it("is exactly the single aggregate tag", () => {
    expect(INVALIDATE_STORE).toEqual([STORE_TAG])
    expect(STORE_TAG).toBe("Store")
  })
})

describe("endpoint classification is complete and disjoint", () => {
  // reactHooksModule binds useQuery/useMutation at module-composition time
  // (not render), so these exist in node — same import-smoke contract as
  // use-sessions-browser.test.ts.
  const all = Object.entries(vaultApi.endpoints)
  const queryNames = all
    .filter(
      ([, e]) =>
        e && typeof (e as { useQuery?: unknown }).useQuery === "function",
    )
    .map(([n]) => n)
  const mutationNames = all
    .filter(
      ([, e]) =>
        e && typeof (e as { useMutation?: unknown }).useMutation === "function",
    )
    .map(([n]) => n)

  it("endpoint detection found queries (guards the useQuery detection)", () => {
    // If this fails, reactHooksModule did not bind hooks in node — fall back to
    // an explicit endpoint-name set in tags.ts and compare names only.
    expect(
      queryNames.length,
      "useQuery detection found no query endpoints",
    ).toBeGreaterThan(0)
  })

  it("every query endpoint is classified as Store-derived OR non-Store", () => {
    const classified = new Set([...STORE_DERIVED_READS, ...NON_STORE_READS])
    for (const n of queryNames) {
      expect(
        classified.has(n),
        `read endpoint "${n}" is unclassified — add it to STORE_DERIVED_READS (and use storeRead()) or NON_STORE_READS in tags.ts`,
      ).toBe(true)
    }
  })

  it("every registered read name is a real query endpoint (guards typos / renames)", () => {
    for (const n of STORE_DERIVED_READS) {
      expect(
        queryNames,
        `${n} is registered but not a query endpoint`,
      ).toContain(n)
    }
    for (const n of NON_STORE_READS) {
      expect(
        queryNames,
        `${n} is registered but not a query endpoint`,
      ).toContain(n)
    }
  })

  it("the two read sets are disjoint", () => {
    for (const n of STORE_DERIVED_READS) {
      expect(NON_STORE_READS, `${n} appears in both read sets`).not.toContain(n)
    }
  })

  it("every declared whole-Store write is a real mutation endpoint", () => {
    for (const n of WHOLE_STORE_WRITES) {
      expect(mutationNames, `${n} is not a mutation endpoint`).toContain(n)
    }
  })

  it("no Store-derived read is also a whole-Store write (disjoint read/write)", () => {
    for (const n of STORE_DERIVED_READS) {
      expect(WHOLE_STORE_WRITES, `${n} should not be a write`).not.toContain(n)
    }
  })
})
