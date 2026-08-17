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
    // 预设栏 = 弹层面（--popover）直接铺开，右侧 border-r hairline 与表单
    // 流分隔（用户决策 2026-08-14 重调：弹层内两栏不再各自成卡，单面结构
    // ——暗色下 card #0d0d0f 比弹层 #26262a 更深，卡片会呈凹洞状）。px-6
    // 与右栏表单对齐。选中 chip 的 tint + 色点语言不变。
    <aside className="border-border flex w-72 shrink-0 flex-col border-r">
      <div className="px-6 pt-4 pb-1 text-sm font-medium">
        {t("providers.presets.title")}
      </div>
      <div className="relative px-6 pt-1">
        <Search className="text-muted-foreground absolute top-1/2 left-7 size-3.5 -translate-y-1/2" />
        <Input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t("providers.presets.searchPlaceholder")}
          aria-label={t("providers.presets.searchPlaceholder")}
          className="h-8 pl-7"
        />
      </div>
      <div className="flex flex-col gap-3 overflow-y-auto px-6 py-3">
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
                      variant="ghost"
                      size="sm"
                      onClick={() => onSelect(preset)}
                      aria-pressed={isSelected}
                      className={cn(
                        "justify-start",
                        // 常态 bg-muted（亮浅灰/暗深灰）——单面弹层上 chip 要
                        // 有「可点面」才立得住（用户决策 2026-08-14：控件与
                        // 弹层同色，看不出哪里可按）。hover 走 --hover 加深/
                        // 提亮，选中态维持 tint + inset shadow 语言不变。
                        "bg-muted",
                        isSelected &&
                          "bg-accent-tint shadow-[inset_0_0_0_1px_var(--accent-brand)]",
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
