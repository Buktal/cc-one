// projectOptions（项目下拉选项派生）的生产路径测试——随 useProjectCandidates
// 一起从 features/usage/derive.test.ts 迁入（架构审查Ⅲ候选①），用例不变。

import { describe, expect, it } from "vitest"

import { projectOptions } from "@/lib/project-candidates"

describe("projectOptions (project dropdown derivation)", () => {
  // A representative sentinel VALUE — the real one arrives as endpoint data
  // (ProjectCandidates.unknown); the derivation only compares values.
  const SENTINEL = "__unknown_project__"
  const PROJECTS = ["/home/u/alpha", "/home/u/beta"]

  it("labels known projects by basename and sorts them", () => {
    const opts = projectOptions(PROJECTS, null, null, "", "未知项目")
    expect(opts).toEqual([
      { value: "/home/u/alpha", label: "alpha" },
      { value: "/home/u/beta", label: "beta" },
    ])
  })

  it("appends the labeled unknown option only while the endpoint offers it", () => {
    const opts = projectOptions(PROJECTS, SENTINEL, SENTINEL, "", "未知项目")
    expect(opts[opts.length - 1]).toEqual({
      value: SENTINEL,
      label: "未知项目",
    })
    // No unknown usage in the window → no special option at all.
    expect(
      projectOptions(PROJECTS, null, null, "", "未知项目").some(
        (o) => o.label === "未知项目",
      ),
    ).toBe(false)
  })

  it("merges a selected value the window dropped (stale known or stale sentinel)", () => {
    // Facet merge-back: a selected project whose sessions left the window
    // stays pickable, basename-labeled.
    const staleKnown = projectOptions(
      PROJECTS,
      null,
      null,
      "/gone/gamma",
      "未知项目",
    )
    expect(staleKnown).toContainEqual({
      value: "/gone/gamma",
      label: "gamma",
    })
    // A selected sentinel whose unknown usage left the window merges back and
    // still reads as the labeled option via the remembered value — never as
    // its raw literal.
    const staleSentinel = projectOptions(
      PROJECTS,
      null,
      SENTINEL,
      SENTINEL,
      "未知项目",
    )
    expect(staleSentinel).toContainEqual({
      value: SENTINEL,
      label: "未知项目",
    })
  })
})
