// CollapseTrigger —— 折叠触发器的行为原语（非视觉件，不违反 shadcn 组件
// 纪律）：role="button" + tabIndex + Enter/Space 切换（preventDefault 挡滚动）
// + aria-expanded 的键盘契约单一归属。为什么不是 <button>：折叠头部普遍内嵌
// 复制按钮等功能 <button>，HTML 禁止按钮嵌套——div/tr 承担同一键盘契约，
// 此前这段契约与逐字相同的 biome-ignore 注释手抄五处跨四文件，收敛于此。
//
// 两个出口共享同一份判定：
// - collapseTriggerProps：props 工厂，展开到任意元素（div 头部、表格行 <tr>
//   ——tbody 里不允许 div，表格行的触发器只能是 tr 本身）；
// - <CollapseTrigger>：div 薄组件（最常见形态的直用面）。
//
// vitest 是 node-only：判定与工厂都是纯函数，直接可测（collapse-trigger.test.ts）。

import type { HTMLAttributes, ReactNode } from "react"

/** Enter / Space 激活折叠切换（React 统一空格键名为 " "）。 */
export function collapseKeyActivates(key: string): boolean {
  return key === "Enter" || key === " "
}

export interface CollapseTriggerOptions {
  /** 展开态；传入即输出 aria-expanded（无开合语义的行级触发器——如「点行
   *  钻入/预览」——可省，不强加 disclosure 语义）。 */
  expanded?: boolean
  onToggle: () => void
  /** 目标守卫：事件目标不是本元素时忽略——行内嵌套控件（输入框 / 按钮）的
   *  Enter 与点击不吃掉行动作（library 行级变体）。默认关（触发器头部
   *  内嵌的功能按钮自己 stopPropagation）。 */
  selfTargetOnly?: boolean
  /** 前置守卫：false 时整份契约不挂载（无 role / 无焦点 / 无处理器）——
   *  「重命名编辑中行不可触发」这类带条件的变体以一个开关表达，不再散成
   *  tabIndex=undefined + 处理器内 return 两处。 */
  enabled?: boolean
}

/** 工厂输出的契约形状——事件参数取结构化最小面（key/target/currentTarget/
 *  preventDefault/stopPropagation），可展开到任意 JSX 元素上。 */
export type CollapseTriggerContract = {
  role: "button"
  tabIndex: number
  "aria-expanded"?: boolean
  onClick: (event: {
    target: unknown
    currentTarget: unknown
    stopPropagation(): void
  }) => void
  onKeyDown: (event: {
    key: string
    target: unknown
    currentTarget: unknown
    preventDefault(): void
  }) => void
}

/** 折叠触发契约 → 可展开的 props。禁用（enabled=false）输出空对象——契约
 *  整体缺席，而不是「挂着但点了没反应」。 */
export function collapseTriggerProps({
  expanded,
  onToggle,
  selfTargetOnly = false,
  enabled = true,
}: CollapseTriggerOptions): CollapseTriggerContract {
  if (!enabled) return {} as CollapseTriggerContract
  const accepts = (target: unknown, currentTarget: unknown): boolean =>
    !selfTargetOnly || target === currentTarget
  return {
    role: "button",
    tabIndex: 0,
    ...(expanded === undefined ? {} : { "aria-expanded": expanded }),
    onClick: (event) => {
      if (!accepts(event.target, event.currentTarget)) return
      onToggle()
    },
    onKeyDown: (event) => {
      if (!accepts(event.target, event.currentTarget)) return
      if (!collapseKeyActivates(event.key)) return
      event.preventDefault()
      onToggle()
    },
  }
}

/** div 形态的直用组件（最常见变体）：工具块 / 气泡折叠头等非表格场景。 */
export function CollapseTrigger({
  expanded,
  onToggle,
  className,
  children,
  ...rest
}: {
  expanded: boolean
  onToggle: () => void
  className?: string
  children: ReactNode
} & Omit<HTMLAttributes<HTMLDivElement>, "onClick" | "onKeyDown">) {
  return (
    <div
      {...rest}
      {...collapseTriggerProps({ expanded, onToggle })}
      className={className}
    >
      {children}
    </div>
  )
}
