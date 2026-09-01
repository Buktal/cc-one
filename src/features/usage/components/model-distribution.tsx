// Model distribution — top-N models by cost or tokens, with an "其他" aggregate.
// Clicking a row narrows the dashboard filter to that model (onPickModel), which
// re-runs every Usage-tagged query including this one (providesTags is
// filter-scoped, so the list itself refreshes too). Rows render through the
// section system's DistRow (#106)：聚合行照系统语义 hatch（与项目区聚合行
// 同一读法）且不可点——onPickModel 只认具体模型。

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
import {
  formatCost,
  formatCount,
  formatMetricLine,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTokens,
} from "@/lib/format"
import { DistRow } from "./dist-row"

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
    request_count: number
    cache_hit_rate?: number
  }> = [
    ...top.map((row) => ({
      label: row.model,
      value: row.value,
      model: row.model,
      request_count: row.request_count,
      cache_hit_rate: row.cache_hit_rate,
    })),
    ...(rest.count > 0
      ? [
          {
            label: t("usage.models.others", { n: rest.count }),
            value: rest.sum,
            model: null,
            request_count: rest.requests,
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
            // DSL: 分布行主值不带标签（模型名在行左即标签）—— `数量 · 占比`；
            // 缓存命中是有标签的段，与主值同行拼接进 value 半行。请求数在
            // sub 行（#119 维度行字段补全：ModelStatsRow 自带、此前未展示）。
            const value = formatMetricLine([
              formatSegValue(fmt(it.value), it.value / total),
              ...(it.cache_hit_rate
                ? [
                    formatMetricSeg(
                      t("usage.models.cacheHit"),
                      formatPct(it.cache_hit_rate),
                    ),
                  ]
                : []),
            ])
            // 当前 filter 选中的模型行高亮 —— 点击行收窄全看板后, 这里
            // 既是反馈 (知道筛选生效) 也是入口 (header chip 一键清除)。
            // 聚合行（model 为 null）不可点——onPickModel 只认具体模型。
            const model = it.model
            return (
              <DistRow
                key={it.label}
                /* 模型 id 行 mono；「其他」是本地化聚合名，同系统里未知项目
                    行一样不 mono。 */
                mono={model != null}
                name={it.label}
                value={value}
                share={it.value / total}
                sub={formatMetricLine([
                  formatMetricSeg(
                    t("usage.hero.requests"),
                    formatCount(it.request_count),
                  ),
                ])}
                hatch={model == null}
                selected={model === filter.model}
                onClick={model == null ? undefined : () => onPickModel(model)}
              />
            )
          })
        )}
      </CardContent>
    </Card>
  )
}
