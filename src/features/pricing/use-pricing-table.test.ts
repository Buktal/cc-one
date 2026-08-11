// Smoke test for the pricing table hook.
//
// The hook's value is wiring pure derivations (filterAndSortPricing /
// nextSortState / paginate) to React state; those derivations are already
// covered in derive.test.ts and pagination.test.ts. vitest runs in a pure node
// environment (see vitest.config.ts — no DOM), so renderHook is out of scope.
// What we can and should guard here is that the module imports cleanly in node
// (it pulls in @/app/store/api → tauri-specta bindings) and still exports the
// expected surface — a regression that moves a Tauri handle fetch to module top
// level would otherwise make the hook un-importable and zero-tested, the same
// failure mode that once hid the shell-hooks bug.

import { describe, expect, it } from "vitest"

describe("usePricingTable imports in a non-Tauri (node) environment", () => {
  it("imports without throwing and exports a function", async () => {
    const mod = await import("./use-pricing-table")
    expect(typeof mod.usePricingTable).toBe("function")
  })

  it("exports the PAGE_SIZE constant the view needs for rendering", async () => {
    const mod = await import("./use-pricing-table")
    expect(mod.PAGE_SIZE).toBe(20)
  })
})
