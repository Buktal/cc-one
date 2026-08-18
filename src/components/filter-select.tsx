// FilterSelect — 筛选下拉的单一实现（收敛了此前 8 份手写变体）。
//
// base-ui Select（shadcn 包装）不收空串 value，于是每份手写变体各自编码同一
// 组规则：空串 ↔ ALL_FILTER 哨兵映射/反映射、「全部」选项置顶、已选值的
// label 解析、空串禁忌。本组件把规则收敛到一处——调用方只接触「空串 =
// 全部」域（value / onChange / options），哨兵完全不漏到组件外。
//
// 哨兵往返映射是纯函数（toSelectValue / fromSelectValue，见
// @/lib/filter-options），filter-options.test.ts 直接断言生产路径。
// 弹出层固定从 trigger 底部往下展开（列表顶对齐 trigger 底，而非把选中项
// 贴齐 trigger 导致列表上下错位）—— shadcn SelectContent 包装的默认行为。

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  type FilterOption,
  fromSelectValue,
  toSelectValue,
} from "@/lib/filter-options"
import { ALL_FILTER } from "@/lib/source-tags"

export interface FilterSelectProps {
  /** 「全部」选项的显示名（trigger 选中态与下拉首项共用）。 */
  allLabel: string
  /** 候选选项；「全部」由组件自动置顶。 */
  options: readonly FilterOption[]
  /** 当前选中值；空串 = 「全部」。 */
  value: string
  /** 值变更回调；空串 = 「全部」。 */
  onChange: (value: string) => void
  /** trigger 的无障碍标签。 */
  ariaLabel?: string
  /** SelectTrigger 的 className（宽度 / 主题样式）。 */
  className?: string
  /** SelectContent 的 className。 */
  contentClassName?: string
  /** 弹层对齐（默认 start；右栏卡片传 end 让菜单向左生长不溢出视口）。 */
  align?: "start" | "end"
  /** 当前值不在 options 里时显示的 label（默认显示原值）。 */
  fallbackLabel?: string
  /** trigger 尺寸（与 shadcn SelectTrigger 的 size 对齐）。 */
  triggerSize?: "sm" | "default"
}

export function FilterSelect({
  allLabel,
  options,
  value,
  onChange,
  ariaLabel,
  className,
  contentClassName,
  align,
  fallbackLabel,
  triggerSize = "default",
}: FilterSelectProps) {
  // 已选值 label 解析：与选项项走同一张 options 表；值不在表里（如分组被
  // 删、设备被重置）回退 fallbackLabel 或原值。
  const labelOf = (v: string) =>
    options.find((o) => o.value === v)?.label ?? fallbackLabel ?? v
  return (
    <Select
      value={toSelectValue(value)}
      onValueChange={(v) => onChange(fromSelectValue(v ?? ""))}
    >
      <SelectTrigger
        className={className}
        size={triggerSize}
        aria-label={ariaLabel}
      >
        <SelectValue className="min-w-0">
          {(val: string) => (val === ALL_FILTER ? allLabel : labelOf(val))}
        </SelectValue>
      </SelectTrigger>
      <SelectContent align={align} className={contentClassName}>
        <SelectItem value={ALL_FILTER}>{allLabel}</SelectItem>
        {options.map((o) => (
          <SelectItem key={o.value} value={o.value}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
