// 确认流程（ConfirmDialog 的 busy / 关闭时序）单一归属。providers / pricing
// / library 三处视图各手写过一份「busy 置位 → await 删除 → 成功才关框」或
// 「先关框再删」的状态流（注释几乎逐句相同），收敛到本 hook：时序不变量落
// 进纯状态机 deleteFlowReducer（可测），hook 只做 React 接线。名字从
// useConfirmDelete 更名 useConfirmAction（架构审查Ⅲ候选⑪）：第四个消费方
// （providers 缺必填切换确认）不是删除，「requestDelete / deleting」在那读
// 不通——流程本质是「先确认再执行的动作」，删除只是最常见的动作。reducer
// 与其测试保持原名原样（deleteFlowReducer 词汇已是历史事实，不为名而改）。
//
// 两种模式（沿用 ConfirmDialog 的既有词汇）：
// - holdOpen（默认）：确认框保持打开直到动作成功——失败留在框内可重试；
//   busy 在成功 / 失败两条路径都复位（关闭由调用方 prop 驱动、不触发
//   onOpenChange，残留 busy 会让下次打开弹窗时按钮一直转圈）。
// - closeFirst（library / providers 切换确认）：确认即关框、动作后台执行
//   ——行内已有 busy spinner 接管（library 的 busyRelPath；providers 的
//   toast），busy 恒为 false。

import { useState } from "react"

/** 删除确认流程的时序状态：target 非空 = 确认框开着；busy = 删除 mutation 在途。 */
export interface DeleteFlowState<T> {
  target: T | null
  busy: boolean
}

/** 流程事件——hook 把 React 回调映射成这些事件喂给 deleteFlowReducer。 */
export type DeleteFlowAction<T> =
  | { kind: "request"; target: T }
  | { kind: "cancel" }
  | { kind: "confirm-start" }
  | { kind: "confirm-done"; ok: boolean }

/** 纯状态迁移（生产路径 = hook 的每一次 setState 都经它）。holdOpen 时
 *  confirm-start 置 busy、confirm-done 成功才清 target（失败留在框内，busy
 *  复位可重试）；closeFirst 时 confirm-start 立即清 target（框关、删除后台
 *  跑），晚到的 confirm-done 因 busy 恒 false 而 no-op。cancel 清除一切——
 *  删除中按钮已 disabled，取消只在空闲态可达。 */
export function deleteFlowReducer<T>(
  state: DeleteFlowState<T>,
  action: DeleteFlowAction<T>,
  holdOpen: boolean,
): DeleteFlowState<T> {
  switch (action.kind) {
    case "request":
      return { target: action.target, busy: false }
    case "cancel":
      return { target: null, busy: false }
    case "confirm-start":
      if (!state.target) return state
      return holdOpen ? { ...state, busy: true } : { target: null, busy: false }
    case "confirm-done":
      if (!state.busy) return state
      return action.ok
        ? { target: null, busy: false }
        : { ...state, busy: false }
  }
}

export function useConfirmAction<T>({
  holdOpen = true,
  onAction,
}: {
  /** 见文件头：holdOpen = 框保持打开直到成功；closeFirst = 确认即关框。 */
  holdOpen?: boolean
  /** 确认后执行的动作；返回是否成功（holdOpen 模式据此决定关框）。 */
  onAction: (target: T) => Promise<boolean>
}) {
  const [state, setState] = useState<DeleteFlowState<T>>({
    target: null,
    busy: false,
  })
  /** 行内触发点 → 打开确认框。 */
  function request(target: T): void {
    setState((s) => deleteFlowReducer(s, { kind: "request", target }, holdOpen))
  }
  /** 取消 / 背景 / ESC → 关框。 */
  function cancel(): void {
    setState((s) => deleteFlowReducer(s, { kind: "cancel" }, holdOpen))
  }
  /** 确认按钮 → 跑动作。holdOpen 模式在完成后按成功与否决定关框。 */
  async function confirm(): Promise<void> {
    if (!state.target) return
    setState((s) => deleteFlowReducer(s, { kind: "confirm-start" }, holdOpen))
    const ok = await onAction(state.target)
    setState((s) =>
      deleteFlowReducer(s, { kind: "confirm-done", ok }, holdOpen),
    )
  }
  return {
    /** 待执行目标（非 null = 确认框开着）。 */
    pending: state.target,
    /** 动作在途（holdOpen 模式；closeFirst 恒 false）。 */
    busy: state.busy,
    request,
    cancel,
    confirm,
  }
}
