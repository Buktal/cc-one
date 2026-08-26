// Hover-intent 状态机回归（architecture.md:「测试必须跑生产路径」——组件
// 只做接线，开合判定跑的就是这份表）。核心守的不变量：事件流静止后至多再走
// 一步 commit 即稳定；反射焦点（base-ui 开时移焦入弹层、关时还焦给按钮）
// 一律恒等，零输入的自持振荡（「反复弹窗」）无路可活。

import { describe, expect, it } from "vitest"

import {
  type HoverIntentEvent,
  type HoverIntentState,
  hoverIntentIdle,
  hoverIntentNext,
} from "./use-hover-intent"

const feed = (
  s: HoverIntentState,
  ...events: HoverIntentEvent[]
): HoverIntentState => events.reduce(hoverIntentNext, s)

describe("hoverIntentNext", () => {
  it("pointer cycle: enter queues open, commit opens; leave queues close, commit closes", () => {
    expect(feed(hoverIntentIdle, "enter")).toEqual({
      open: false,
      pending: "open",
    })
    const opened = feed(hoverIntentIdle, "enter", "commit")
    expect(opened).toEqual({ open: true, pending: null })
    expect(feed(opened, "leave")).toEqual({ open: true, pending: "close" })
    expect(feed(opened, "leave", "commit")).toEqual(hoverIntentIdle)
  })

  it("latest intent wins: enter cancels a scheduled close and vice versa", () => {
    expect(feed({ open: true, pending: "close" }, "enter")).toEqual({
      open: true,
      pending: null,
    })
    expect(feed(hoverIntentIdle, "enter", "leave")).toEqual(hoverIntentIdle)
  })

  it("reflected focus is identity — the closed-by-choreography reopen loop stays dead", () => {
    // 历史病灶全轨迹：hover 打开 → 离开收起后 base-ui 把焦点还给触发按钮，
    // 还焦点亮 onFocus 重开 —— 断言收起完成后的还焦什么也不改变。
    const closedAgain = feed(
      hoverIntentIdle,
      "enter",
      "commit",
      "leave",
      "commit",
    )
    expect(closedAgain).toEqual(hoverIntentIdle)

    let s = closedAgain
    for (let i = 0; i < 50; i++) s = feed(s, "focus-reflected")
    expect(s).toEqual(closedAgain)
  })

  it("blur into the popup body is internal choreography, not leaving", () => {
    const opened = feed(hoverIntentIdle, "enter", "commit")
    expect(feed(opened, "blur-inside")).toEqual(opened)
  })

  it("keyboard focus opens like hover; leaving focus closes like leave", () => {
    expect(feed(hoverIntentIdle, "enter-keyboard", "commit")).toEqual({
      open: true,
      pending: null,
    })
    expect(
      feed({ open: true, pending: null }, "enter-keyboard", "blur-outside"),
    ).toEqual({ open: true, pending: "close" })
  })

  it("dismiss settles instantly from any state, clearing any pending flip", () => {
    expect(feed({ open: false, pending: "open" }, "dismiss")).toEqual(
      hoverIntentIdle,
    )
    expect(feed({ open: true, pending: "close" }, "dismiss")).toEqual(
      hoverIntentIdle,
    )
  })

  it("commit without a pending flip is a no-op", () => {
    expect(feed(hoverIntentIdle, "commit")).toEqual(hoverIntentIdle)
    expect(feed({ open: true, pending: null }, "commit")).toEqual({
      open: true,
      pending: null,
    })
  })
})
