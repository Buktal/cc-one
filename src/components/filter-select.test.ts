// Tests for the FilterSelect component.
//
// vitest runs in a pure node environment (see vitest.config.ts — no DOM), so
// the component's behavior is guarded through its pure internals in
// @/lib/filter-options (filter-options.test.ts — sentinel round-trip, label
// resolution, facet merge-back); what we guard here is that the module imports
// cleanly in node (it pulls the base-ui Select wrapper) — the same failure
// mode the browser hooks guard against (a regression that hoists a DOM handle
// to module top level would make the component un-importable and zero-tested).

import { describe, expect, it } from "vitest"

describe("FilterSelect imports in a non-DOM (node) environment", () => {
  it("imports without throwing and exports a component", async () => {
    const mod = await import("@/components/filter-select")
    expect(typeof mod.FilterSelect).toBe("function")
  })
})
