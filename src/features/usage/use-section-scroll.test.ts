import { describe, expect, it } from "vitest"

import { pickActiveSection } from "@/features/usage/use-section-scroll"

describe("pickActiveSection", () => {
  it("picks the LAST section whose top has scrolled past the edge", () => {
    const tops = [
      { id: "overview", top: 300 },
      { id: "projects", top: 74 },
      { id: "sessions", top: 10 },
      { id: "requests", top: -50 },
    ]
    expect(pickActiveSection(tops, 74, "overview")).toBe("requests")
  })

  it("keeps the fallback while no section has crossed the edge yet", () => {
    const tops = [
      { id: "overview", top: 500 },
      { id: "projects", top: 620 },
    ]
    expect(pickActiveSection(tops, 74, "overview")).toBe("overview")
  })

  it("boundary hit counts as active (top == edge)", () => {
    expect(
      pickActiveSection([{ id: "projects", top: 74 }], 74, "overview"),
    ).toBe("projects")
  })
})
