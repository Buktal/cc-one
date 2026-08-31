// InlineTextEdit —— 行内文本编辑器的呈现端原子件（行为原语，非视觉新件：
// 面仍是 ui/input + ui/button）。键盘 / 失焦 / 按钮的提交契约此前在 library
// 的行内重命名编辑器里手写了 52 行（device-list 的变体缺一半契约），收敛到
// 这里一次做对，消费方只给值与回调：
//
// - Enter / 失焦 / ✓ 三路提交共用 requestCommit 一个口：busy 挡二次提交，
//   空草稿（trim 后）不可提交——失焦在空草稿时转为放弃（收起），不留一个
//   游离的空编辑器；
// - Escape / ✕ = 取消（弃草稿收起）；
// - ✓/✕ 在 mousedown preventDefault：点击按钮不夺焦点、输入框不失焦——
//   否则确认键会「失焦提交 + 点击提交」双发，取消键会先提交再取消。
//
// 与 use-inline-edit 配对使用（draft/busy/commit/cancel 状态机归它）；不依赖
// 该 hook，直接喂值与回调也可用。

import { Check, Loader2, X } from "lucide-react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

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
}: {
  /** 当前草稿（受控）。 */
  value: string
  /** 输入 → 改草稿（useInlineEdit 的 setDraft）。 */
  onValueChange: (value: string) => void
  /** 提交在途：✓ 转圈并禁用，三路提交全部挡下。 */
  busy?: boolean
  /** 提交（草稿非空且空闲时才会被触发）。 */
  onCommit: () => void
  /** 取消（Escape / ✕ / 空草稿失焦）。 */
  onCancel: () => void
  /** 包裹层的 className（宽度 / 高度跟宿主布局，如表格列、设备行）。 */
  className?: string
  /** 输入框的追加 className（覆盖默认 h-7）。 */
  inputClassName?: string
  placeholder?: string
  ariaLabel?: string
  autoFocus?: boolean
}) {
  const canSubmit = value.trim().length > 0
  // 唯一提交口：Enter / blur / ✓ 全走这里，契约只写一遍。
  function requestCommit() {
    if (!busy && canSubmit) onCommit()
  }
  // 失焦即了结：可提交则提交，空草稿视为放弃（同 library 既有契约——
  // 点击空白收起，而不是留一个游离编辑器）。
  function onBlur() {
    if (canSubmit) requestCommit()
    else onCancel()
  }
  return (
    <div className={cn("relative", className)}>
      <Input
        value={value}
        onChange={(e) => onValueChange(e.target.value)}
        className={cn("h-7 w-full pr-16", inputClassName)}
        placeholder={placeholder}
        aria-label={ariaLabel}
        autoFocus={autoFocus}
        onKeyDown={(e) => {
          if (e.key === "Enter") requestCommit()
          if (e.key === "Escape") onCancel()
        }}
        onBlur={onBlur}
      />
      <div className="absolute top-1/2 right-1 flex -translate-y-1/2 gap-0.5">
        <Button
          variant="ghost"
          size="icon-sm"
          disabled={busy || !canSubmit}
          onMouseDown={(e) => e.preventDefault()}
          onClick={requestCommit}
        >
          {busy ? <Loader2 className="animate-spin" /> : <Check />}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          onMouseDown={(e) => e.preventDefault()}
          onClick={onCancel}
        >
          <X />
        </Button>
      </div>
    </div>
  )
}
