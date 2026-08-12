// 预设选择器（供应商「新增」Sheet 的左栏）：内置预设的分组 chip 列表 +
// 搜索。从原 PresetSidePanel 改造而来——去掉了 fixed 悬浮定位（旧版遮挡主
// 列表、靠 calc(60vw) 硬算贴表单左缘、且 slide-in-from-right 动画方向与其实
// 际出现在表单左侧的位置相反）。现在是 Sheet 内的流式左侧栏，与右侧表单
// 成组，由 Sheet 统一开关。点预设即把它的 settings.json 快照预填进表单
// （预设只是起点，可连续切换覆盖）。仅在有内置预设的 app 显示——opencode
// 附加模式 presetsForApp 返回空，父级不挂载本组件。
//
// 分组逻辑复用 presetsForApp（单一事实来源）。

import { Search } from "lucide-react"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  type ProviderPreset,
  presetsForApp,
} from "@/features/providers/presets"
import { cn } from "@/lib/utils"

import type { App, ProviderCategory } from "@/types/generated/bindings"

/** 分组展示顺序：官方 → 云厂商 → 聚合。 */
const CATEGORY_ORDER: ProviderCategory[] = [
  "official",
  "cloud_provider",
  "aggregator",
]

export function PresetPicker({
  app,
  selected,
  onSelect,
}: {
  app: App
  /** 当前表单已选中的预设（新建态；连续切换覆盖）。 */
  selected?: ProviderPreset | null
  onSelect: (preset: ProviderPreset) => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")

  const groups = useMemo(() => {
    const all = presetsForApp(app)
    const q = query.trim().toLowerCase()
    const matches = q
      ? all.filter((p) => p.name.toLowerCase().includes(q))
      : all
    return CATEGORY_ORDER.map((category) => ({
      category,
      // cn_official 与 official 显示同名（界面不按地域区分官方供应商），
      // 预设并入「官方」组不单列；DB 里存的 category 值仍是 cn_official。
      presets: matches.filter((p) =>
        category === "official"
          ? p.category === "official" || p.category === "cn_official"
          : p.category === category,
      ),
    })).filter((group) => group.presets.length > 0)
  }, [query, app])

  return (
    <aside className="border-border flex w-72 shrink-0 flex-col border-r">
      <div className="border-border px-3 border-b py-2 text-sm font-medium">
        {t("providers.presets.title")}
      </div>
      <div className="relative px-3 pt-2">
        <Search className="text-muted-foreground absolute top-1/2 left-5 size-3.5 -translate-y-1/2" />
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("providers.presets.searchPlaceholder")}
          aria-label={t("providers.presets.searchPlaceholder")}
          className="h-8 pl-7"
        />
      </div>
      <div className="flex flex-col gap-3 overflow-y-auto p-3">
        {groups.length === 0 ? (
          <div className="text-muted-foreground py-4 text-center text-xs">
            {t("providers.presets.noMatch")}
          </div>
        ) : (
          groups.map((group) => (
            <div key={group.category} className="flex flex-col gap-1.5">
              <div className="text-muted-foreground text-xs">
                {t(`providers.category.${group.category}`)}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {group.presets.map((preset) => {
                  const isSelected = preset.name === selected?.name
                  return (
                    <Button
                      key={preset.name}
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => onSelect(preset)}
                      aria-pressed={isSelected}
                      className={cn(
                        "justify-start",
                        // 选中态与 OptionCards 决策卡同一套视觉语言：inset shadow
                        // 描边 + 淡色底，字体颜色保持不动（border-* 自定义色会被
                        // outline variant 的 border-border 覆盖，见 option-cards）。
                        isSelected &&
                          "border-transparent bg-accent-tint shadow-[inset_0_0_0_1px_var(--accent-brand)] dark:border-transparent dark:bg-accent-tint",
                      )}
                    >
                      {preset.iconColor ? (
                        <span
                          aria-hidden
                          className="size-2 shrink-0 rounded-full"
                          style={{ backgroundColor: preset.iconColor }}
                        />
                      ) : null}
                      <span className="truncate">{preset.name}</span>
                    </Button>
                  )
                })}
              </div>
            </div>
          ))
        )}
      </div>
    </aside>
  )
}
