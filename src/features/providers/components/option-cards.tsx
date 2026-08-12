// OptionCards — 通用「选项决策卡」（radiogroup 卡片选择）。
//
// 选项平铺可见可对比、语义写进卡片内部——下拉藏选项 + 行下动态小字在切换时
// 会文字跳变、撑高布局。JSON 迁移弹窗的冲突处理 / 密钥选项、CC-Switch 导入的
// 冲突处理共用这一份视觉语言（单一事实来源，不各抄一份）。
// 选项与语义文案由调用方传入：共享的导入模式选项在 IMPORT_MODE_OPTIONS，
// 弹窗私有选项（如导出密钥）就地定义。

import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { ProviderImportMode } from "@/types/generated/bindings"

/** 一张决策卡的选项：value + 主文案 i18n 键 + 可选卡片内语义小字 i18n 键。 */
export type OptionCardOption<T extends string> = Readonly<{
  value: T
  labelKey: string
  hintKey?: string
}>

/** 共享选项：导入冲突处理（合并 / 覆盖）——JSON 迁移与 CC-Switch 导入共用。 */
export const IMPORT_MODE_OPTIONS: ReadonlyArray<
  OptionCardOption<ProviderImportMode>
> = [
  {
    value: "merge",
    labelKey: "providers.transfer.mode.merge",
    hintKey: "providers.transfer.mode.mergeHint",
  },
  {
    value: "overwrite",
    labelKey: "providers.transfer.mode.overwrite",
    hintKey: "providers.transfer.mode.overwriteHint",
  },
]

export function OptionCards<T extends string>({
  value,
  onValueChange,
  options,
  ariaLabel,
}: {
  value: T
  onValueChange: (value: T) => void
  options: ReadonlyArray<OptionCardOption<T>>
  ariaLabel: string
}) {
  const { t } = useTranslation()
  return (
    <div
      role="radiogroup"
      aria-label={ariaLabel}
      className="grid grid-cols-2 gap-2"
    >
      {options.map((option) => {
        const selected = value === option.value
        return (
          <Button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={selected}
            variant="outline"
            onClick={() => onValueChange(option.value)}
            className={cn(
              "h-auto flex-col items-start gap-0.5 rounded-md py-2 text-left font-normal whitespace-normal",
              // 选中只换主题色描边 + 淡色底——字体颜色保持不动。描边用 inset
              // shadow（1px 内描边）而非 border-accent-brand：outline variant 的
              // border-border / dark:border-input 会把自定义色 border 类覆盖掉
              //（tailwind-merge 把 border-accent-brand 当 width 类处理），shadow
              // 不参与该冲突（与 ProviderRow 激活行的色条同一模式）。
              selected &&
                "border-transparent bg-accent-tint shadow-[inset_0_0_0_1px_var(--accent-brand)] dark:border-transparent dark:bg-accent-tint",
            )}
          >
            <span className="text-sm">{t(option.labelKey)}</span>
            {option.hintKey ? (
              <span className="text-muted-foreground text-xs font-normal">
                {t(option.hintKey)}
              </span>
            ) : null}
          </Button>
        )
      })}
    </div>
  )
}
