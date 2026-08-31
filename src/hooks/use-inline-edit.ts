// 行内编辑（draft + open + busy + 提交/取消）的单一归属。这段小机器此前手抄
// 三份跨两个 feature——library 行内重命名、settings 显示名、device-list 设备
// 名——三份的时序契约逐字相同：开编辑器时抓快照做草稿、提交期间挡二次提交、
// 成功才收起（关闭并清空草稿）、失败留在编辑态可改、取消随时可达且让晚到的
// 完成事件 no-op。收敛到本 hook 后，时序不变量落进纯状态机 inlineEditNext
// （可测），hook 只做 React 接线（同 use-confirm-action 的结构）。
//
// target 是编辑目标的键位（library 传整个 LibraryEntry，消费方按 rel_path 比
// 较；device-list 传 DeviceInfo）——开编辑器那一刻抓到的对象让提交回调天然
// 拿到原始值（如改前的名字），无需再查表。无键位的常开编辑（settings 的
// 显示名输入框）以 K = void 使用：begin / cancel / target 不参与，机器退化
// 为「草稿 + busy + 成功清空」。

import { useState } from "react"

/** 编辑器状态：target 非空 = 编辑器开着（keyed 变体）；draft 为当前草稿；
 *  busy = 提交在途。 */
export interface InlineEditState<K> {
  target: K | null
  draft: string
  busy: boolean
}

/** 流程事件——hook 把 React 回调映射成这些事件喂给 inlineEditNext。 */
export type InlineEditAction<K> =
  | { kind: "begin"; target: K; draft: string }
  | { kind: "edit"; draft: string }
  | { kind: "commit-start" }
  | { kind: "commit-done"; ok: boolean }
  | { kind: "cancel" }

/** 空闲闭态：取消 / 成功收起都回到这里（草稿一并清空，下次 begin 重抓）。 */
export function inlineEditClosed<K>(): InlineEditState<K> {
  return { target: null, draft: "", busy: false }
}

/** 纯状态迁移（生产路径 = hook 的每一次 setState 都经它）。
 *  - commit-start 在 busy 时 no-op：✓/Enter/blur 三路提交共用一个在途位，
 *    双击或「blur 提交 + 点击 ✓」不会发出第二份变更；
 *  - commit-done 只在 busy 时生效：cancel 已让编辑器收起的场合，晚到的完成
 *    事件不再把状态拉回去（取消赢）；成功才收起（ok=false 留在编辑态可改）；
 *  - cancel 无条件回闭态——在途的提交不撤回（后端仍会落地），只保证界面
 *    收起且晚到事件无效。 */
export function inlineEditNext<K>(
  state: InlineEditState<K>,
  action: InlineEditAction<K>,
): InlineEditState<K> {
  switch (action.kind) {
    case "begin":
      return { target: action.target, draft: action.draft, busy: false }
    case "edit":
      return { ...state, draft: action.draft }
    case "commit-start":
      if (state.busy) return state
      return { ...state, busy: true }
    case "commit-done":
      if (!state.busy) return state
      return action.ok ? inlineEditClosed() : { ...state, busy: false }
    case "cancel":
      return inlineEditClosed()
  }
}

export function useInlineEdit<K>(options: {
  /** 提交回调：resolve true = 成功（收起并清空草稿）；false = 失败（留在
   *  编辑态，草稿保留可改）。busy 期间的重复调用由机器挡下，回调不会收到。 */
  commit: (target: K, draft: string) => Promise<boolean>
}) {
  const [state, setState] = useState<InlineEditState<K>>(inlineEditClosed)

  /** 打开编辑器：绑定位移目标 + 抓初始草稿快照。 */
  function begin(target: K, draft: string): void {
    setState((s) => inlineEditNext(s, { kind: "begin", target, draft }))
  }

  /** 输入 → 改草稿。 */
  function setDraft(draft: string): void {
    setState((s) => inlineEditNext(s, { kind: "edit", draft }))
  }

  /** 取消（Esc / ✕ / 点空白放弃）：收起、弃草稿。 */
  function cancel(): void {
    setState((s) => inlineEditNext(s, { kind: "cancel" }))
  }

  /** 提交当前草稿；按回调结果决定收起或留在编辑态。busy 中重复触发是
   *  no-op（机器挡下），调用方无需自判。 */
  async function commit(): Promise<void> {
    if (state.busy) return
    setState((s) => inlineEditNext(s, { kind: "commit-start" }))
    const ok = await options.commit(state.target as K, state.draft)
    setState((s) => inlineEditNext(s, { kind: "commit-done", ok }))
  }

  return {
    /** 编辑目标（keyed 变体：非 null = 开着；常开变体不读它）。 */
    target: state.target,
    /** 当前草稿。 */
    draft: state.draft,
    /** 提交在途（✓ 转圈 / 挡二次提交 / 禁其它动作由调用方接线）。 */
    busy: state.busy,
    begin,
    setDraft,
    commit,
    cancel,
  }
}
