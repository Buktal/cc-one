// Hover-intent 浮层开合控制器 —— 开合规则的唯一实现 + 计时接线。
//
// 不变量（表驱动守住）：只有真实指针动作、真实键盘聚焦或用户显式关闭能改变
// 开合；开合编排自身反射回来的焦点事件一律恒等。历史背景（NarrowStatsTrigger
// 的「反复弹窗」）：触发按钮曾把 onMouseEnter 与 onFocus 接进同一套延时器，
// 而 base-ui 在混合过点击路径后会接管焦点——打开时移焦入弹层、关闭时还焦给
// 按钮——还原出来的 focus 再次点亮 onFocus 重开计时器：开→关→还焦→再开的
// 零输入自持振荡。修法是让任何一次翻转都必须由新的外部事件驱动：事件流静止
// 后，至多再走一步 commit 即归于稳定，不存在无输入也能延续的环路。

import { useEffect, useReducer } from "react"

/** 进触发器/弹层的悬停确认延迟（略过路过的指针）。 */
export const HOVER_OPEN_MS = 100
/** 离开缓冲：穿过触发器与弹层之间的缝隙不收卡。 */
export const HOVER_CLOSE_MS = 180

export type HoverIntentEvent =
  // 真实意图：指针进入、键盘聚焦、指针离开、焦点去往外处、用户显式关闭
  // （Esc / 点外面 / 点击切换的关闭方向）。
  | "enter"
  | "enter-keyboard"
  | "leave"
  | "blur-outside"
  | "dismiss"
  // 内部编排反射：焦点管理器在触发器与弹层之间搬运焦点（开时移入 /
  // 关时还原 / Tab 落进弹层内部）。这些不是用户的意愿表达，恒等处理。
  | "focus-reflected"
  | "blur-inside"
  // pending 计时器到期的提交。
  | "commit"

export interface HoverIntentState {
  /** 浮层当前开关。 */
  open: boolean
  /** 待执行的翻转；null = 静止（此后无外部事件则永不再变）。 */
  pending: "open" | "close" | null
}

export const hoverIntentIdle: HoverIntentState = { open: false, pending: null }

export function hoverIntentNext(
  s: HoverIntentState,
  e: HoverIntentEvent,
): HoverIntentState {
  switch (e) {
    case "enter":
    case "enter-keyboard":
      // 已开（含 close 定时在途）＝留在原地并撤掉收起排程；未开＝排队亮出。
      return s.open ? { open: true, pending: null } : { ...s, pending: "open" }
    case "leave":
    case "blur-outside":
      // 已开（含 open 定时在途）＝排队收起；未开＝原地不动并撤掉亮出排程。
      return s.open ? { ...s, pending: "close" } : hoverIntentIdle
    case "focus-reflected":
    case "blur-inside":
      return s
    case "dismiss":
      return hoverIntentIdle
    case "commit":
      if (s.pending === "open") return { open: true, pending: null }
      if (s.pending === "close") return { open: false, pending: null }
      return s
  }
}

/** 组件侧接线：pending 出现即起定时器，翻转或换向自动作废旧定时。开合
 *  判定全部走 hoverIntentNext——组件里不再手写 setTimeout / 清理逻辑。 */
export function useHoverIntent(): {
  open: boolean
  advance: (e: HoverIntentEvent) => void
} {
  const [state, advance] = useReducer(hoverIntentNext, hoverIntentIdle)
  useEffect(() => {
    if (!state.pending) return
    const timer = setTimeout(
      () => advance("commit"),
      state.pending === "open" ? HOVER_OPEN_MS : HOVER_CLOSE_MS,
    )
    return () => clearTimeout(timer)
  }, [state.pending])
  return { open: state.open, advance }
}
