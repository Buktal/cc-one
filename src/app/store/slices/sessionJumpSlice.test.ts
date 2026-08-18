import { describe, expect, it } from "vitest"

import sessionJumpReducer, {
  clearSessionJump,
  type SessionJumpTarget,
  setSessionJumpTarget,
} from "@/app/store/slices/sessionJumpSlice"

const target: SessionJumpTarget = { id: "sid-1", device_id: "dev-a" }

describe("sessionJumpSlice", () => {
  it("starts with no target (no pending jump survives app start)", () => {
    expect(sessionJumpReducer(undefined, { type: "@@INIT" }).target).toBeNull()
  })

  it("set stores the target; clear resets it (write-then-consume cycle)", () => {
    let s = sessionJumpReducer(undefined, setSessionJumpTarget(target))
    expect(s.target).toEqual(target)
    s = sessionJumpReducer(s, clearSessionJump())
    expect(s.target).toBeNull()
  })

  it("a second jump to the same session stores a fresh target object", () => {
    // 消费方以 target 引用身份触发 effect：同会话重复跳转必须产生新对象，
    // 否则第二次跳转会被认为「无变化」而吞掉。
    const first = sessionJumpReducer(undefined, setSessionJumpTarget(target))
    const second = sessionJumpReducer(
      first,
      setSessionJumpTarget({ ...target }),
    )
    expect(second.target).not.toBe(first.target)
    expect(second.target).toEqual(first.target)
  })
})
