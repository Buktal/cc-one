// 预设选择器（供应商视图中部）：18 个内置预设的 chip 网格。按名称搜索、按
// category 分组展示。点选一个 chip 即把该预设整份交给父级，父级用它在表单里
// 预填一份 settingsConfig 快照——预设常量本身绝不被改动。

import { Search } from "lucide-react"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
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

export function PresetSelector({
  app,
  onSelect,
}: {
  app: App
  onSelect: (preset: ProviderPreset) => void
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

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-2">
        <CardTitle>{t("providers.presets.title")}</CardTitle>
        <div className="relative w-56">
          <Search className="text-muted-foreground absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("providers.presets.searchPlaceholder")}
            aria-label={t("providers.presets.searchPlaceholder")}
            className="pl-8"
          />
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
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
      </CardContent>
    </Card>
  )
}
