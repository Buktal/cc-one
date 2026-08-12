// ImportModeCards — 「冲突处理」决策卡（合并 / 覆盖）。
//
// JSON 迁移（ProviderTransferDialog）与 CC-Switch 导入（CcSwitchImportDialog）
// 两个入口共用：选项平铺可见可对比、语义写进卡片内部——下拉藏选项 + 行下
// 动态小字在切换时会文字跳变、撑高布局。选项与语义文案的单一事实来源：
// MODE_OPTIONS / 卡片 JSX 只在这里定义，两个弹窗不再各抄一份。

import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"
import type { ProviderImportMode } from "@/types/generated/bindings"

const MODE_OPTIONS: ReadonlyArray<[ProviderImportMode, string]> = [
  ["merge", "providers.transfer.mode.merge"],
  ["overwrite", "providers.transfer.mode.overwrite"],
]

/** 冲突策略 → 卡片内语义小字的 i18n key（每张卡自带解释，并列对比）。 */
const MODE_HINT: Record<ProviderImportMode, string> = {
  merge: "providers.transfer.mode.mergeHint",
  overwrite: "providers.transfer.mode.overwriteHint",
}

export function ImportModeCards({
  value,
  onValueChange,
}: {
  value: ProviderImportMode
  onValueChange: (value: ProviderImportMode) => void
}) {
  const { t } = useTranslation()
  return (
    <div
      role="radiogroup"
      aria-label={t("providers.transfer.mode.label")}
      className="grid grid-cols-2 gap-2"
    >
      {MODE_OPTIONS.map(([mode, key]) => {
        const selected = value === mode
        return (
          <Button
            key={mode}
            type="button"
            role="radio"
            aria-checked={selected}
            variant="outline"
            onClick={() => onValueChange(mode)}
            className={cn(
              "h-auto flex-col items-start gap-0.5 rounded-md py-2 text-left font-normal whitespace-normal",
              selected && "border-accent-brand bg-accent-tint",
            )}
          >
            <span
              className={cn(
                "text-sm",
                selected && "text-accent-brand-strong font-medium",
              )}
            >
              {t(key)}
            </span>
            <span className="text-muted-foreground text-xs font-normal">
              {t(MODE_HINT[mode])}
            </span>
          </Button>
        )
      })}
    </div>
  )
}
