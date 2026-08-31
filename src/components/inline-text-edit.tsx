// InlineTextEdit —— 行内文本编辑器的呈现端原子件（行为原语，非视觉新件：
// 面仍是 ui/input + ui/button）。键盘 / 失焦 / 按钮的提交契约此前在 library
// 的行内重命名编辑器里手写了 52 行（device-list 的变体缺一半契约），收敛到
// 这里一次做对，消费方只给值与回调：
//
// - Enter / 失焦 / ✓ 三路了结共用 requestFinish 一个口，决策是纯函数
//   inlineEditFinish（可测）：busy 在途挡一切了结——✓ 置灰、Enter/blur 的
//   二次触发是 no-op，双发不出第二份变更；空草稿（trim 后）不可提交——
//   Enter / 失焦在空草稿时都转为放弃（收起），不留一个游离的空编辑器；
// - Escape / ✕ = 取消（弃草稿收起）。Escape 同时置内部取消位：即便取消后
//   编辑器卸载前还有 blur 派发（或宿主延迟收起），晚到的 blur 也不会把
//   Escape 前的草稿提交上去——「取消不双发」由本件承载，消费方不再自写
//   守卫；宿主未卸载而用户重新输入时复位取消位；
// - ✓/✕ 在 mousedown preventDefault：点击按钮不夺焦点、输入框不失焦——
//   否则确认键会「失焦提交 + 点击提交」双发，取消键会先提交再取消。
//
// 与 use-inline-edit 配对使用（draft/busy/commit/cancel 状态机归它）；不依赖
// 该 hook，直接喂值与回调也可用。bare 形态（不渲染 ✓/✕ 按钮，输入框不预留
// 按钮位）给「名字就地改写、行内没有按钮位」的场景（live-import 条目行）：
// 键盘/失焦契约与完整形态是同一份。

import { Check, Loader2, X } from "lucide-react"
import { useRef } from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

/** 三路了结（Enter / blur / ✓）的纯决策——requestFinish 的每次触发都跑它
 *  （architecture.md: "测试必须跑生产路径"）。优先级：已取消或 busy 在途 =
 *  ignore（取消后晚到的 blur、在途中的二次触发都被挡下）；空闲 + 草稿可提交
 *  = commit；空闲 + 空草稿 = abandon（收起而非提交）。 */
export type InlineEditFinish = "commit" | "abandon" | "ignore"

export function inlineEditFinish(p: {
  /** 提交在途（宿主接 useInlineEdit 时即其 busy）。 */
  busy: boolean
  /** Escape 已取消——晚到的 blur 不再了结。 */
  cancelled: boolean
  /** 草稿 trim 后非空。 */
  canSubmit: boolean
}): InlineEditFinish {
  if (p.cancelled || p.busy) return "ignore"
  return p.canSubmit ? "commit" : "abandon"
}

export function InlineTextEdit({
  value,
  onValueChange,
  busy = false,
  onCommit,
  onCancel,
  className,
  inputClassName,
  placeholder,
  ariaLabel,
  autoFocus = false,
  bare = false,
  selectOnFocus = false,
}: {
  /** 当前草稿（受控）。 */
  value: string
  /** 输入 → 改草稿（useInlineEdit 的 setDraft）。 */
  onValueChange: (value: string) => void
  /** 提交在途：✓ 转圈并禁用，三路了结全部挡下。 */
  busy?: boolean
  /** 提交（草稿非空且空闲时才会被触发）。 */
  onCommit: () => void
  /** 取消（Escape / ✕ / 空草稿的 Enter 或失焦）。 */
  onCancel: () => void
  /** 包裹层的 className（宽度 / 高度跟宿主布局，如表格列、设备行）。 */
  className?: string
  /** 输入框的追加 className（覆盖默认 h-7）。 */
  inputClassName?: string
  placeholder?: string
  ariaLabel?: string
  autoFocus?: boolean
  /** bare 形态：不渲染 ✓/✕ 按钮，输入框也不预留按钮位；键盘/失焦契约不变
   *  （提交靠 Enter/失焦，放弃靠 Escape/空草稿）。 */
  bare?: boolean
  /** 聚焦时全选草稿——「点开即整体改写」的编辑场景（live-import 条目行）。 */
  selectOnFocus?: boolean
}) {
  const canSubmit = value.trim().length > 0
  // Escape 置位的取消位：晚到 blur 的了结口据此挡下。ref 只需活到编辑器
  // 卸载——各消费方取消即收起卸载。
  const cancelledRef = useRef(false)
  // 唯一了结口：Enter / blur / ✓ 全走这里，契约只写一遍。
  function requestFinish() {
    const r = inlineEditFinish({
      busy,
      cancelled: cancelledRef.current,
      canSubmit,
    })
    if (r === "commit") onCommit()
    else if (r === "abandon") onCancel()
  }
  function requestCancel() {
    cancelledRef.current = true
    onCancel()
  }
  return (
    <div className={cn("relative", className)}>
      <Input
        value={value}
        onChange={(e) => {
          cancelledRef.current = false
          onValueChange(e.target.value)
        }}
        onFocus={selectOnFocus ? (e) => e.currentTarget.select() : undefined}
        className={cn("h-7 w-full", !bare && "pr-16", inputClassName)}
        placeholder={placeholder}
        aria-label={ariaLabel}
        autoFocus={autoFocus}
        onKeyDown={(e) => {
          if (e.key === "Enter") requestFinish()
          if (e.key === "Escape") requestCancel()
        }}
        onBlur={requestFinish}
      />
      {!bare && (
        <div className="absolute top-1/2 right-1 flex -translate-y-1/2 gap-0.5">
          <Button
            variant="ghost"
            size="icon-sm"
            disabled={busy || !canSubmit}
            onMouseDown={(e) => e.preventDefault()}
            onClick={requestFinish}
          >
            {busy ? <Loader2 className="animate-spin" /> : <Check />}
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onMouseDown={(e) => e.preventDefault()}
            onClick={requestCancel}
          >
            <X />
          </Button>
        </div>
      )}
    </div>
  )
}
