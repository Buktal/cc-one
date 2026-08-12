// 预设侧栏面板（供应商「新增」时紧挨右侧编辑表单滑入）：内置预设的分组
// chip 列表。镜像会话详情的「主面板 + turn-nav」双面板结构——新增供应商时，
// 右侧编辑表单与紧挨它左侧的预设面板同时展开（成组在右侧），点预设即把它
// 的 settings.json 快照预填进表单（预设只是起点，可连续切换覆盖）。仅在
// 有内置预设的 app 显示：opencode 是附加模式，presetsForApp 返回空，面板
// 不挂载。
//
// 复用 presetsForApp 的分组逻辑（单一事实来源）；与旧版常驻预设卡片相比，
// 退居为「新增」流程的辅助面板，让供应商列表成为页面主角。

import { Search, X } from "lucide-react"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  type ProviderPreset,
  presetsForApp,
} from "@/features/providers/presets"

import type { App, ProviderCategory } from "@/types/generated/bindings"

/** 分组展示顺序：官方 → 云厂商 → 国内官方 → 聚合。 */
const CATEGORY_ORDER: ProviderCategory[] = [
  "official",
  "cloud_provider",
  "cn_official",
  "aggregator",
]

export function PresetSidePanel({
  app,
  open,
  onSelect,
  onClose,
}: {
  app: App
  open: boolean
  onSelect: (preset: ProviderPreset) => void
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [query, setQuery] = useState("")

  const groups = useMemo(() => {
    // 预设按应用分流：claude 18 / codex 17 / gemini 6，单一事实来源走 presetsForApp。
    const all = presetsForApp(app)
    const q = query.trim().toLowerCase()
    const matches = q
      ? all.filter((p) => p.name.toLowerCase().includes(q))
      : all
    return CATEGORY_ORDER.map((category) => ({
      category,
      presets: matches.filter((p) => p.category === category),
    })).filter((group) => group.presets.length > 0)
  }, [query, app])

  // open 由父级按「新增模式 + 该 app 有内置预设」控制；无预设的 app（opencode
  // 附加模式）父级直接传 false。搜索无匹配时 groups 为空，交由下方 noMatch 渲染。
  if (!open) return null

  return (
    // 紧挨右侧编辑表单（表单宽 60vw，从 right-0 出）的左边缘、留 0.75rem 间隙——
    // 两个面板成组在右侧（镜像会话详情的 主面板 + turn-nav），不贴窗口左缘。
    <nav
      aria-label={t("providers.presets.title")}
      className="fixed top-1/2 z-[60] w-80 -translate-y-1/2 animate-in slide-in-from-right duration-200"
      style={{ right: "calc(60vw + 0.75rem)" }}
    >
      <div className="flex max-h-[calc(100vh-4rem)] flex-col rounded-lg border border-border bg-popover shadow-lg">
        <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-2">
          <span className="text-sm font-medium">
            {t("providers.presets.title")}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("common.close")}
            onClick={onClose}
          >
            <X />
          </Button>
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
                  {group.presets.map((preset) => (
                    <Button
                      key={preset.name}
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => onSelect(preset)}
                      className="justify-start"
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
                  ))}
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </nav>
  )
}
