// fetch 拉到的模型列表下拉（共享小组件）：三处（claude / gemini / opencode）
// 同一结构——Select + font-mono 项 + placeholder 兼作 trigger 的 aria-label。
// 差异只有文案键、onPick 与可选的宽度约束（claude 的 max-w-sm）。空列表不
// 渲染（调用方无需各自判长度）。

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

export function ModelPickSelect({
  models,
  placeholder,
  onPick,
  className,
}: {
  models: string[]
  /** 兼作 trigger 的 aria-label 与 SelectValue 的占位文案。 */
  placeholder: string
  onPick: (model: string) => void
  /** 可选的外层宽度约束（claude 的 max-w-sm）；不传则无包装，直接渲染
   *  Select（保持 gemini / opencode 原 DOM）。 */
  className?: string
}) {
  if (models.length === 0) return null
  const select = (
    <Select
      onValueChange={(model) => {
        if (typeof model === "string") onPick(model)
      }}
    >
      <SelectTrigger className="font-mono text-xs" aria-label={placeholder}>
        <SelectValue placeholder={placeholder} />
      </SelectTrigger>
      <SelectContent>
        {models.map((model) => (
          <SelectItem key={model} value={model} className="font-mono text-xs">
            {model}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
  return className ? <div className={className}>{select}</div> : select
}
