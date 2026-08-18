// 哨兵往返映射与 facet 候选派生的行为测试——FilterSelect 组件内部用的就是
// 这两个纯函数，这里断言的是生产路径本身（「选了模型再切时间窗，下拉不空」
// 从手点验证落成自动化断言）。

import { describe, expect, it } from "vitest"

import { ALL_FILTER } from "@/lib/source-tags"

import { facetOptions, fromSelectValue, toSelectValue } from "./filter-options"

describe("哨兵往返映射 (toSelectValue / fromSelectValue)", () => {
  it("空串映射为 ALL_FILTER 哨兵，反映射回空串", () => {
    expect(toSelectValue("")).toBe(ALL_FILTER)
    expect(fromSelectValue(ALL_FILTER)).toBe("")
  })

  it("非空值原样通过", () => {
    expect(toSelectValue("claude_code")).toBe("claude_code")
    expect(fromSelectValue("claude_code")).toBe("claude_code")
  })

  it("任意值往返后不变（Select 拿到的值送回 onChange 即回到调用方域）", () => {
    for (const v of ["", "claude_code", "gemini_cli"]) {
      expect(fromSelectValue(toSelectValue(v))).toBe(v)
    }
  })
})

describe("facetOptions（已选值并回候选）", () => {
  it("已选值不在候选里也并回——切时间窗后下拉不空、选中不丢", () => {
    // 选了 glm-4.7 再切到另一个时间窗：新窗口的 distinct 候选中没有它。
    expect(facetOptions(["claude-code", "grok-code"], "glm-4.7")).toEqual([
      "claude-code",
      "glm-4.7",
      "grok-code",
    ])
  })

  it("已选值已在候选中则去重不重复", () => {
    expect(facetOptions(["b", "a"], "a")).toEqual(["a", "b"])
  })

  it("未选任何值时候选原样排序返回", () => {
    expect(facetOptions(["b", "a"], "")).toEqual(["a", "b"])
  })

  it("候选为空时只回已选值（或空）", () => {
    expect(facetOptions([], "x")).toEqual(["x"])
    expect(facetOptions([], "")).toEqual([])
  })
})
