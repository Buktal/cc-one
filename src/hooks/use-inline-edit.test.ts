// 行内编辑状态机测试：inlineEditNext 就是 useInlineEdit 每一次 setState 跑的
// 那份代码（architecture.md: "测试必须跑生产路径"）。vitest 是 node-only
// 环境（无 renderHook，见 vitest.config.ts），时序不变量断言在纯迁移函数上：
// 三路提交共用一个在途位、成功才收起、取消赢过晚到的完成事件。

import { describe, expect, it } from "vitest"

import {
  type InlineEditState,
  inlineEditClosed,
  inlineEditNext,
} from "./use-inline-edit"

const open = (draft: string): InlineEditState<string> => ({
  target: "row-1",
  draft,
  busy: false,
})

describe("begin / edit", () => {
  it("begin 绑定目标并抓草稿快照，不处于 busy", () => {
    expect(
      inlineEditNext(inlineEditClosed<string>(), {
        kind: "begin",
        target: "row-1",
        draft: "a.md",
      }),
    ).toEqual({
      target: "row-1",
      draft: "a.md",
      busy: false,
    })
  })

  it("edit 只改草稿，目标与 busy 不动", () => {
    const s = inlineEditNext(open("a.md"), { kind: "edit", draft: "b.md" })
    expect(s).toEqual({ target: "row-1", draft: "b.md", busy: false })
  })
})

describe("提交时序（三路提交共用一个在途位）", () => {
  it("commit-start 置 busy", () => {
    expect(inlineEditNext(open("b.md"), { kind: "commit-start" }).busy).toBe(
      true,
    )
  })

  it("busy 中再 commit-start 是 no-op（双击 / blur+✓ 不发第二份变更）", () => {
    const busy = inlineEditNext(open("b.md"), { kind: "commit-start" })
    expect(inlineEditNext(busy, { kind: "commit-start" })).toBe(busy)
  })

  it("消费方路径：在途时第二次提交被挡（session-detail 标题改名的双 Enter 只落一份写入）", () => {
    let s = inlineEditNext(inlineEditClosed<string>(), {
      kind: "begin",
      target: "row-1",
      draft: "b.md",
    })
    s = inlineEditNext(s, { kind: "edit", draft: "c.md" })
    // 第一次提交进在途位；第二次提交（Enter / 保存键再触发）被同一在途位挡下
    // ——状态对象不变，即第二份 commit-start 没有发生。
    const inFlight = inlineEditNext(s, { kind: "commit-start" })
    expect(inlineEditNext(inFlight, { kind: "commit-start" })).toBe(inFlight)
  })

  it("commit-done 成功 → 收起并清空草稿（成功后关闭语义）", () => {
    const busy = inlineEditNext(open("b.md"), { kind: "commit-start" })
    expect(inlineEditNext(busy, { kind: "commit-done", ok: true })).toEqual(
      inlineEditClosed(),
    )
  })

  it("commit-done 失败 → 留在编辑态，草稿保留可改，busy 复位", () => {
    const busy = inlineEditNext(open("b.md"), { kind: "commit-start" })
    expect(inlineEditNext(busy, { kind: "commit-done", ok: false })).toEqual({
      target: "row-1",
      draft: "b.md",
      busy: false,
    })
  })
})

describe("取消时序（取消赢过晚到的完成事件）", () => {
  it("cancel 无条件回闭态（含 busy 中取消——在途提交不撤回，只收界面）", () => {
    const busy = inlineEditNext(open("b.md"), { kind: "commit-start" })
    expect(inlineEditNext(busy, { kind: "cancel" })).toEqual(inlineEditClosed())
  })

  it("cancel 后晚到的 commit-done 是 no-op（不把状态拉回去）", () => {
    const busy = inlineEditNext(open("b.md"), { kind: "commit-start" })
    const cancelled = inlineEditNext(busy, { kind: "cancel" })
    expect(inlineEditNext(cancelled, { kind: "commit-done", ok: true })).toBe(
      cancelled,
    )
  })
})
