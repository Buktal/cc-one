// CollapseTrigger 原语的纯函数测试：键盘判定（collapseKeyActivates）与契约
// 工厂（collapseTriggerProps）就是五处调用点运行的那份代码（architecture.md:
// "测试必须跑生产路径"）。node-only 环境无 DOM，事件用结构化最小面伪造。

import { describe, expect, it, vi } from "vitest"

import {
  type CollapseTriggerContract,
  collapseKeyActivates,
  collapseTriggerProps,
} from "./collapse-trigger"

/** 伪造一个只带契约所需字段的事件；currentTarget 默认等于 target（即
 *  「目标是触发器本身」）。 */
function keyEvent(
  key: string,
  { target = "self" }: { target?: unknown } = {},
): {
  key: string
  target: unknown
  currentTarget: unknown
  preventDefault: ReturnType<typeof vi.fn>
} {
  return {
    key,
    target,
    currentTarget: "self",
    preventDefault: vi.fn(),
  }
}

/** 把工厂输出的事件处理器按契约签名调用的小桥（测试只关心行为）。 */
function fireKeyDown(
  contract: CollapseTriggerContract,
  key: string,
  target?: unknown,
): { preventDefault: ReturnType<typeof vi.fn> } {
  const event = keyEvent(key, { target })
  contract.onKeyDown(event)
  return { preventDefault: event.preventDefault }
}

describe("collapseKeyActivates", () => {
  it("Enter 与空格激活", () => {
    expect(collapseKeyActivates("Enter")).toBe(true)
    expect(collapseKeyActivates(" ")).toBe(true)
  })

  it("其余键不激活（方向键滚页、字符输入等）", () => {
    expect(collapseKeyActivates("ArrowDown")).toBe(false)
    expect(collapseKeyActivates("a")).toBe(false)
    expect(collapseKeyActivates("Spacebar")).toBe(false)
  })
})

describe("collapseTriggerProps", () => {
  it("契约面：role + tabIndex + aria-expanded + 点击/键盘都切 onToggle", () => {
    const onToggle = vi.fn()
    const contract = collapseTriggerProps({ expanded: true, onToggle })
    expect(contract.role).toBe("button")
    expect(contract.tabIndex).toBe(0)
    expect(contract["aria-expanded"]).toBe(true)

    contract.onClick({
      target: "self",
      currentTarget: "self",
      stopPropagation: vi.fn(),
    })
    expect(onToggle).toHaveBeenCalledTimes(1)

    fireKeyDown(contract, "Enter")
    fireKeyDown(contract, " ")
    expect(onToggle).toHaveBeenCalledTimes(3)
  })

  it("非激活键不切换也不 preventDefault（方向键要留给滚动）", () => {
    const onToggle = vi.fn()
    const contract = collapseTriggerProps({ expanded: false, onToggle })
    const { preventDefault } = fireKeyDown(contract, "ArrowDown")!
    expect(onToggle).not.toHaveBeenCalled()
    expect(preventDefault).not.toHaveBeenCalled()
  })

  it("激活键 preventDefault（挡住 Space 的页面滚动语义）", () => {
    const contract = collapseTriggerProps({
      expanded: false,
      onToggle: vi.fn(),
    })
    const { preventDefault } = fireKeyDown(contract, " ")!
    expect(preventDefault).toHaveBeenCalledTimes(1)
  })

  it("selfTargetOnly：目标不是本元素时忽略（行内嵌套控件的键/点击不吃掉）", () => {
    const onToggle = vi.fn()
    const contract = collapseTriggerProps({
      expanded: false,
      onToggle,
      selfTargetOnly: true,
    })
    fireKeyDown(contract, "Enter", "inner-input")
    expect(onToggle).not.toHaveBeenCalled()
    fireKeyDown(contract, "Enter", "self")
    expect(onToggle).toHaveBeenCalledTimes(1)

    contract.onClick({
      target: "inner-input",
      currentTarget: "self",
      stopPropagation: vi.fn(),
    })
    expect(onToggle).toHaveBeenCalledTimes(1)
    contract.onClick({
      target: "self",
      currentTarget: "self",
      stopPropagation: vi.fn(),
    })
    expect(onToggle).toHaveBeenCalledTimes(2)
  })

  it("缺省不加目标守卫（触发器内部的 span 点击仍触发——行级表格行靠它）", () => {
    const onToggle = vi.fn()
    const contract = collapseTriggerProps({ expanded: false, onToggle })
    contract.onClick({
      target: "cell-span",
      currentTarget: "tr",
      stopPropagation: vi.fn(),
    })
    expect(onToggle).toHaveBeenCalledTimes(1)
  })

  it("enabled=false → 空契约（无 role / 无焦点位 / 无处理器）", () => {
    const contract = collapseTriggerProps({
      expanded: false,
      onToggle: vi.fn(),
      enabled: false,
    })
    expect(contract).toEqual({})
  })

  it("expanded 缺省不输出 aria-expanded（无开合语义的行级触发器不强加）", () => {
    const contract = collapseTriggerProps({ onToggle: vi.fn() })
    expect("aria-expanded" in contract).toBe(false)
    expect(contract.role).toBe("button")
  })
})
