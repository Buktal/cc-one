// Controller tests for the paged browser (架构扫描候选⑧): `pagedBrowserNext`
// (pure state machine) and `scopeKeyOf` (scope identity) are the exact code
// the hook runs — vitest's node-only environment can't render the hook, so the
// three rules (scope-change reset / page↔offset / post-delete clamp) are
// asserted on the production transition function itself.

import { describe, expect, it } from "vitest"

import {
  type PagedBrowserState,
  pagedBrowserNext,
  scopeKeyOf,
} from "./use-paged-browser"

const PAGE_SIZE = 20
const initial: PagedBrowserState = { offset: 0, scopeKey: null }

describe("scope 身份变化 → 回第 1 页（结构性规则）", () => {
  it("换维度（身份变化）→ offset 归 0 并记录新身份", () => {
    let s = pagedBrowserNext(
      initial,
      { kind: "go-to-page", page: 3 },
      PAGE_SIZE,
    )
    expect(s.offset).toBe(40)
    s = pagedBrowserNext(
      s,
      { kind: "scope-sync", scopeKey: "new-scope" },
      PAGE_SIZE,
    )
    expect(s.offset).toBe(0)
    expect(s.scopeKey).toBe("new-scope")
  })

  it("身份不变 → offset 保持（不产生伪重置）", () => {
    let s = pagedBrowserNext(
      initial,
      { kind: "scope-sync", scopeKey: "same" },
      PAGE_SIZE,
    )
    s = pagedBrowserNext(s, { kind: "go-to-page", page: 2 }, PAGE_SIZE)
    expect(s.offset).toBe(20)
    const after = pagedBrowserNext(
      s,
      { kind: "scope-sync", scopeKey: "same" },
      PAGE_SIZE,
    )
    expect(after).toBe(s) // 同一引用 → React 跳过重渲染
    expect(after.offset).toBe(20)
  })

  it("新增筛选维度自动改变身份——无需手列依赖清单", () => {
    // 同一份维度集合加一个字段（如新增 device 筛选），身份必须变；否则
    // 「换维度回第 1 页」的护栏就靠不住。
    expect(scopeKeyOf({ source: "all", model: "gpt" })).not.toBe(
      scopeKeyOf({ source: "all" }),
    )
  })

  it("维度值不变 → 身份稳定", () => {
    const a = scopeKeyOf({ search: "abc", model: "gpt", device: "" })
    const b = scopeKeyOf({ search: "abc", model: "gpt", device: "" })
    expect(a).toBe(b)
  })
})

describe("页 ↔ offset 换算", () => {
  it("go-to-page 按 1-based 页换算 offset", () => {
    expect(
      pagedBrowserNext(initial, { kind: "go-to-page", page: 1 }, PAGE_SIZE)
        .offset,
    ).toBe(0)
    expect(
      pagedBrowserNext(initial, { kind: "go-to-page", page: 2 }, PAGE_SIZE)
        .offset,
    ).toBe(20)
    expect(
      pagedBrowserNext(initial, { kind: "go-to-page", page: 3 }, PAGE_SIZE)
        .offset,
    ).toBe(40)
  })

  it("页号 < 1 夹到第 1 页", () => {
    expect(
      pagedBrowserNext(initial, { kind: "go-to-page", page: 0 }, PAGE_SIZE)
        .offset,
    ).toBe(0)
    expect(
      pagedBrowserNext(initial, { kind: "go-to-page", page: -1 }, PAGE_SIZE)
        .offset,
    ).toBe(0)
  })

  it("shift-pages 按页平移并夹到 0（detail-sheet 跨页 prev/next 语义）", () => {
    let s = pagedBrowserNext(
      initial,
      { kind: "go-to-page", page: 3 },
      PAGE_SIZE,
    )
    expect(s.offset).toBe(40)
    s = pagedBrowserNext(s, { kind: "shift-pages", delta: -1 }, PAGE_SIZE)
    expect(s.offset).toBe(20)
    s = pagedBrowserNext(s, { kind: "shift-pages", delta: 1 }, PAGE_SIZE)
    expect(s.offset).toBe(40)
    // 第 1 页再向前 → 夹到 0，offset 不会为负
    s = pagedBrowserNext(s, { kind: "shift-pages", delta: -3 }, PAGE_SIZE)
    expect(s.offset).toBe(0)
  })
})

describe("删除后夹紧", () => {
  it("删掉最后一页的最后一行 → 夹到新最后一页开头", () => {
    // 41 行 / 3 页，当前第 3 页（offset 40）；删 1 行剩 40 → 夹到第 2 页开头。
    let s = pagedBrowserNext(
      initial,
      { kind: "go-to-page", page: 3 },
      PAGE_SIZE,
    )
    s = pagedBrowserNext(s, { kind: "clamp", total: 40 }, PAGE_SIZE)
    expect(s.offset).toBe(20)
  })

  it("删除后仍在页内 → offset 不动", () => {
    let s = pagedBrowserNext(
      initial,
      { kind: "go-to-page", page: 2 },
      PAGE_SIZE,
    )
    s = pagedBrowserNext(s, { kind: "clamp", total: 25 }, PAGE_SIZE)
    expect(s.offset).toBe(20)
  })

  it("缩到空列表 → 夹到 0", () => {
    let s = pagedBrowserNext(
      initial,
      { kind: "go-to-page", page: 2 },
      PAGE_SIZE,
    )
    s = pagedBrowserNext(s, { kind: "clamp", total: 0 }, PAGE_SIZE)
    expect(s.offset).toBe(0)
  })

  it("夹紧只压不抬：总行数扩大不改变当前 offset", () => {
    let s = pagedBrowserNext(
      initial,
      { kind: "go-to-page", page: 1 },
      PAGE_SIZE,
    )
    s = pagedBrowserNext(s, { kind: "clamp", total: 999 }, PAGE_SIZE)
    expect(s.offset).toBe(0)
  })
})
