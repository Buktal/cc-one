// Model distribution — top-N models by cost or tokens, with an "其他" aggregate.
// Clicking a row narrows the dashboard filter to that model (onPickModel), which
// re-runs every Usage-tagged query including this one (providesTags is
// filter-scoped, so the list itself refreshes too).

import { X } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useModelsQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { topNModels } from "@/features/usage/derive"
import { formatCost, formatPct, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"

const TOP_N = 5

export function ModelDistribution({
  filter,
  onPickModel,
  onClearModel,
}: {
  filter: FilterState
  onPickModel: (model: string) => void
  /** 清除当前模型筛选 (header chip 的 ×)。 */
  onClearModel: () => void
}) {
  const { t } = useTranslation()
  const { data: rows = [] } = useModelsQuery(filter)
  // 默认按 tokens 展示 (token-first cockpit), 开关同样 tokens 在前。
  const [metric, setMetric] = useState<"cost" | "tokens">("tokens")

  const fmt = metric === "cost" ? formatCost : formatTokens
  const { top, rest, total } = topNModels(rows, metric, TOP_N)
  const items: Array<{
    label: string
    value: number
    model: string | null
    cache_hit_rate?: number
  }> = [
    ...top.map((row) => ({
      label: row.model,
      value: row.value,
      model: row.model,
      cache_hit_rate: row.cache_hit_rate,
    })),
    ...(rest.count > 0
      ? [
          {
            label: t("usage.models.others", { n: rest.count }),
            value: rest.sum,
            model: null,
          },
        ]
      : []),
  ]

  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.models.title")}</CardTitle>
        {/* 筛选 chip + 指标开关一起放进 CardAction (说明见下)。 */}
        <CardAction>
          <div className="flex items-center gap-2">
            {filter.model ? (
              <button
                type="button"
                onClick={onClearModel}
                aria-label={t("usage.models.clearFilter")}
                className="bg-accent-tint text-accent-brand-strong hover:bg-accent-tint/70 inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring/40"
              >
                <span className="max-w-32 truncate font-mono">
                  {filter.model}
                </span>
                <X className="size-3 shrink-0" />
              </button>
            ) : null}
            {/* 指标开关: header 的 has-[card-action] 会切到
                grid-cols-[1fr_auto], 开关待在自己的 auto 宽列里、justify-self-end
                右对齐, 永不被标题宽度拉伸 (否则英文长标题会把胶囊背景撑出一截空隙)。
                与同目录 usage-trend-chart 的 header 写法一致。 */}
            <div className="bg-muted/60 inline-flex items-center gap-0.5 rounded-md p-0.5">
              {(["tokens", "cost"] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => setMetric(m)}
                  className={`rounded-[5px] px-2 py-0.5 text-xs font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring/40 ${
                    metric === m
                      ? "bg-accent-tint text-accent-brand-strong shadow-sm"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  {m === "tokens"
                    ? t("usage.models.tokens")
                    : t("usage.models.cost")}
                </button>
              ))}
            </div>
          </div>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {items.length === 0 ? (
          <span className="text-muted-foreground text-sm">
            {t("usage.models.empty")}
          </span>
        ) : (
          items.map((it) => {
            const pct = (it.value / total) * 100
            // 当前 filter 选中的模型行高亮 —— 点击行收窄全看板后, 这里
            // 既是反馈 (知道筛选生效) 也是入口 (header chip 一键清除)。
            const selected = it.model === filter.model
            return (
              <button
                key={it.label}
                type="button"
                disabled={!it.model}
                aria-pressed={it.model ? selected : undefined}
                onClick={() => it.model && onPickModel(it.model)}
                className={cn(
                  "group -mx-2 flex flex-col gap-1 rounded-md px-2 py-1.5 text-left disabled:cursor-default",
                  selected ? "bg-accent-tint" : it.model && "hover:bg-hover",
                )}
              >
                <div className="flex items-center justify-between gap-2 text-xs">
                  <span
                    className={cn(
                      "text-foreground truncate font-mono",
                      selected
                        ? "text-accent-brand-strong"
                        : "group-hover:text-primary",
                    )}
                  >
                    {it.label}
                  </span>
                  <span className="text-muted-foreground shrink-0 tabular-nums">
                    {fmt(it.value)} · {formatPct(it.value / total)}
                    {/* 缓存命中率只在有命中 (rate > 0) 时显示; "其他" 聚合行
                        没有后端算好的 rate, 同样不显示。 */}
                    {it.cache_hit_rate ? (
                      <>
                        {" · "}
                        {t("usage.models.cacheHit")}{" "}
                        {formatPct(it.cache_hit_rate)}
                      </>
                    ) : null}
                  </span>
                </div>
                <div className="bg-muted h-1.5 w-full overflow-hidden rounded-full">
                  <div
                    className="bg-primary h-full rounded-full transition-all group-hover:bg-primary/80"
                    style={{ width: `${Math.max(pct, 2)}%` }}
                  />
                </div>
              </button>
            )
          })
        )}
      </CardContent>
    </Card>
  )
}
