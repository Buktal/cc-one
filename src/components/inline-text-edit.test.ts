// InlineTextEdit 三路了结的纯决策测试：inlineEditFinish 就是 Enter / blur / ✓
// 每次触发跑的那份代码（architecture.md: "测试必须跑生产路径"）。vitest 是
// node-only（无 DOM，见 vitest.config.ts），键盘/失焦的时序不变量断言在纯
// 决策上：busy 挡双发（bare 形态的消费方——live-import 条目行——没有 ✓ 可
// 置灰，Enter/失焦是仅有的了结路，同样被挡）、Escape 后晚到的 blur 不提交、
// 空草稿 = 放弃收起。
import { describe, expect, it } from "vitest"

import { inlineEditFinish } from "./inline-text-edit"

const idle = { busy: false, cancelled: false }

describe("三路了结决策（Enter / blur / ✓ 共用）", () => {
  it("空闲 + 草稿可提交 → commit", () => {
    expect(inlineEditFinish({ ...idle, canSubmit: true })).toBe("commit")
  })

  it("空闲 + 空草稿 → abandon（收起，不提交）", () => {
    expect(inlineEditFinish({ ...idle, canSubmit: false })).toBe("abandon")
  })

  it("busy 在途 → ignore（Enter/blur/✓ 的二次触发不发第二份提交）", () => {
    expect(
      inlineEditFinish({ busy: true, cancelled: false, canSubmit: true }),
    ).toBe("ignore")
    expect(
      inlineEditFinish({ busy: true, cancelled: false, canSubmit: false }),
    ).toBe("ignore")
  })

  it("Escape 已取消 → ignore（晚到的 blur 不把取消前的草稿提交上去）", () => {
    expect(
      inlineEditFinish({ busy: false, cancelled: true, canSubmit: true }),
    ).toBe("ignore")
    expect(
      inlineEditFinish({ busy: false, cancelled: true, canSubmit: false }),
    ).toBe("ignore")
  })

  it("取消位优先于 busy（两条挡发路殊途同归）", () => {
    expect(
      inlineEditFinish({ busy: true, cancelled: true, canSubmit: true }),
    ).toBe("ignore")
  })
})
