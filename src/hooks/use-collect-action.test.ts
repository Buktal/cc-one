// collectLabelKey — the collect-button label selection shared by the shell
// sidebar / top-bar buttons and the request-log empty-state CTA. Both surfaces
// used to compose the "collecting?" conditional separately; the in-flight
// branch (multiDevice-aware) is the single source here.

import { describe, expect, it } from "vitest"

import { collectLabelKey } from "@/hooks/use-collect-action"

describe("collectLabelKey", () => {
  it("idle: the surface idle copy wins (sidebar 'sync/collect' vs CTA 'collectLocal')", () => {
    expect(collectLabelKey(false, false, "usage.collect.collect")).toBe(
      "usage.collect.collect",
    )
    expect(collectLabelKey(false, true, "usage.collect.sync")).toBe(
      "usage.collect.sync",
    )
    expect(collectLabelKey(false, false, "usage.collect.collectLocal")).toBe(
      "usage.collect.collectLocal",
    )
  })

  it("collecting: in-flight copy is multiDevice-aware (Syncing… vs Collecting…)", () => {
    expect(collectLabelKey(true, true, "x")).toBe("usage.collect.syncing")
    expect(collectLabelKey(true, false, "x")).toBe("usage.collect.collecting")
  })
})
