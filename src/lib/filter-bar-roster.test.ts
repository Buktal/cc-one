// filterBarRoster——五维筛选条名册（顺序 + 门控）的生产路径直测。渲染在
// @/components/filter-bar 按此名册进行，这里的断言就是装配约定本身。

import { describe, expect, it } from "vitest"

import { filterBarRoster } from "@/lib/filter-bar-roster"

describe("filterBarRoster", () => {
  it("全量名册顺序固定：时间 · 来源 · 模型 · 项目 · 设备", () => {
    expect(filterBarRoster({ hasSources: true, showDevice: true })).toEqual([
      "date",
      "source",
      "model",
      "project",
      "device",
    ])
  })

  it("来源门控：ALL_TIME distinct 为空时来源整颗缺席，其余顺序不变", () => {
    expect(filterBarRoster({ hasSources: false, showDevice: true })).toEqual([
      "date",
      "model",
      "project",
      "device",
    ])
  })

  it("设备门控：会话非收藏轨（showDevice=false）设备整颗缺席", () => {
    expect(filterBarRoster({ hasSources: true, showDevice: false })).toEqual([
      "date",
      "source",
      "model",
      "project",
    ])
  })

  it("双门控同关：剩余名册仍是 时间 · 模型 · 项目", () => {
    expect(filterBarRoster({ hasSources: false, showDevice: false })).toEqual([
      "date",
      "model",
      "project",
    ])
  })
})
