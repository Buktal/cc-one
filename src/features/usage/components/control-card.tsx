// ControlCard / ControlBar — shared meta-controls for the
// data views. Time range · model · source · device filters only. The collect /
// sync action and the data-freshness hint moved to the sidebar (single entry
// point — see shell.tsx); these are pure filter surfaces now. Solid flat
// (no glass / no glow) — Pixso dark.
//
// 横排 ControlBar 的 chip 走 bar (纯值 + 选中「全部」时显全称「全部模型 /
// 全部来源 / 全部设备」自带身份, 与库一致), 纵卡 ControlCard 靠左 Row label,
// chip 只显「全部」. 来源 (source) 维度在多来源 (sources.length > 0) 时才出现
// —— 采到任意来源就显示, 与设备维度同理.

import { ChevronDown } from "lucide-react"
import { type ReactNode, useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  useDistinctModelsQuery,
  useDistinctSourcesQuery,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { ALL_TIME_FILTER, patchFilter } from "@/app/store/slices/filterSlice"
import { DateRangeChip as SharedDateRangeChip } from "@/components/date-range-chip"
import { FilterSelect } from "@/components/filter-select"
import { Button } from "@/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { useDateRangeFilter } from "@/hooks/use-date-range-filter"
import { facetOptions } from "@/lib/filter-options"
import { usePersistedState } from "@/lib/persistence"
import { cn } from "@/lib/utils"
import { sourceLabel } from "../source-labels"
import { useDeviceOptions } from "../use-device-options"
import { DeviceScopeControl } from "./device-scope-control"

const CONTROL_COLLAPSE_KEY = "cc-one:control-collapsed"

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3 py-1">
      <span className="text-muted-foreground shrink-0 text-xs">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  )
}

/** 日期范围 chip —— 把 Redux filterSlice 适配成受控共享组件 (ControlCard 默认
 *  右对齐, ControlBar 左对齐). 数据语义与 sessions 工具栏版一致: 动态预设
 *  (today/7d/30d) 只存 preset、不存具体日期 (日期在 queryFn 实时派生);
 *  日历选日期转 custom 并存具体值. 共享的 JSX / 标签拼装 / 预设清单在
 *  @/components/date-range-chip, slice 读写经 useDateRangeFilter 单一归属
 *  (补丁形状在 filterSlice 的 presetPatch / dayPatch). */
function DateRangeChip({ align = "end" }: { align?: "start" | "end" }) {
  const range = useDateRangeFilter()
  return (
    <SharedDateRangeChip
      preset={range.preset}
      fromDay={range.fromDay}
      toDay={range.toDay}
      onPreset={range.onPreset}
      onFromDay={range.onFromDay}
      onToDay={range.onToDay}
      align={align}
    />
  )
}

function ModelChip({
  align = "start",
  bar = false,
}: {
  align?: "start" | "end"
  /** 横排 ControlBar: 选中「全部」时显全称「全部模型」自带身份。 */
  bar?: boolean
}) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  // Facet filter = 看板筛选去掉 model 维度本身。模型下拉只列「所选时间 / 来源
  // / 设备窗口内真正出现过的模型」, 不按 model 自身收窄 (否则选了 glm 下拉就
  // 只剩 glm); 当前已选模型并回候选, 避免切到没用过它的窗口时 chip 变成空值。
  // 候选跨天滚动靠采集间隔 → usage_changed → invalidate: 动态预设的
  // filter 一天内引用稳定, 无需 dayStr() 触发器。
  const facetFilter = useMemo(() => ({ ...filter, model: "" }), [filter])
  const { data: models = [] } = useDistinctModelsQuery(facetFilter)
  // 并回规则（已选模型并入候选，窗口切换后下拉不空）收敛在 facetOptions。
  const options = useMemo(
    () =>
      facetOptions(models, filter.model).map((m) => ({ value: m, label: m })),
    [models, filter.model],
  )
  const allLabel = bar ? t("usage.control.allModel") : t("usage.control.all")
  return (
    <FilterSelect
      ariaLabel={t("usage.control.model")}
      allLabel={allLabel}
      options={options}
      value={filter.model}
      onChange={(v) => dispatch(patchFilter({ model: v }))}
      className={cn(
        "border-border bg-card hover:bg-hover h-8 w-36 rounded-md",
        // 模型名最长且不可控 → 横排 (bar) 给最宽。
        bar && "w-48",
      )}
      align={align}
    />
  )
}

/** 来源 (source) 维度筛选 — 与 ModelChip 对称, 选项来自 queryDistinctSources. */
function SourceChip({
  align = "start",
  bar = false,
}: {
  align?: "start" | "end"
  /** 横排 ControlBar: 选中「全部」时显全称「全部来源」自带身份。 */
  bar?: boolean
}) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  // 与 ModelChip 对称: facet 去掉 source 自身, 候选只含所选窗口内出现过的来源;
  // 已选来源并回候选。跨天滚动靠采集间隔刷新, 见 ModelChip。
  const facetFilter = useMemo(() => ({ ...filter, source: "" }), [filter])
  const { data: sources = [] } = useDistinctSourcesQuery(facetFilter)
  // 并回规则（已选来源并入候选，窗口切换后下拉不空）收敛在 facetOptions。
  const options = useMemo(
    () =>
      facetOptions(sources, filter.source).map((s) => ({
        value: s,
        label: sourceLabel(s),
      })),
    [sources, filter.source],
  )
  const allLabel = bar ? t("usage.control.allSource") : t("usage.control.all")
  return (
    <FilterSelect
      ariaLabel={t("usage.control.source")}
      allLabel={allLabel}
      options={options}
      value={filter.source}
      onChange={(v) => dispatch(patchFilter({ source: v }))}
      className={cn(
        // 纵卡 ControlCard 内与应用/设备下拉统一 w-36（模型名最长的容纳
        // 宽度，三下拉等宽对齐）；横排 ControlBar 与 sessions 的来源/设备
        // 下拉同宽 w-30。长名称由 line-clamp-1 截断。
        "border-border bg-card hover:bg-hover h-8 w-36 rounded-md",
        bar && "w-30",
      )}
      align={align}
    />
  )
}

/** 纵向卡片版 — 看板右栏。标题行带主题切换 + 折叠。Filters only — the
 *  collect action lives in the sidebar now. */
export function ControlCard() {
  const { t } = useTranslation()
  const multiDevice = useDeviceOptions().length > 0
  const { data: sources = [] } = useDistinctSourcesQuery(ALL_TIME_FILTER)
  const hasSources = sources.length > 0
  // Collapse persists across restarts (debounced write, flushed on unmount).
  const [collapsed, setCollapsed] = usePersistedState<boolean>(
    CONTROL_COLLAPSE_KEY,
    false,
  )
  return (
    <Card size="sm" interactive>
      <CardHeader>
        <CardTitle>{t("usage.control.title")}</CardTitle>
        <CardAction>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={
              collapsed
                ? t("usage.control.expand")
                : t("usage.control.collapse")
            }
            onClick={() => setCollapsed((c) => !c)}
          >
            <ChevronDown
              className={cn(
                "size-4 transition-transform",
                collapsed && "-rotate-90",
              )}
            />
          </Button>
        </CardAction>
      </CardHeader>
      {collapsed ? null : (
        <CardContent className="flex flex-col gap-0">
          <Row label={t("usage.control.dateRange")}>
            <DateRangeChip />
          </Row>
          {hasSources ? (
            <Row label={t("usage.control.source")}>
              <SourceChip align="end" />
            </Row>
          ) : null}
          <Row label={t("usage.control.model")}>
            <ModelChip align="end" />
          </Row>
          {multiDevice ? (
            <Row label={t("usage.deviceScope.label")}>
              <DeviceScopeControl align="end" />
            </Row>
          ) : null}
        </CardContent>
      )}
    </Card>
  )
}

/** 横向条版 — 日志页顶部。Filters only — the collect action lives in the
 *  sidebar now. Two groups in a fixed order: the date range anchors the first
 *  line; the model / source / device chips take their own line on narrow
 *  containers (w-full) and return inline on wide ones (@60rem:w-auto). Fold
 *  measures the bar's own width (@container), so the sidebar's collapsed
 *  state can't shift it. */
export function ControlBar() {
  const { data: sources = [] } = useDistinctSourcesQuery(ALL_TIME_FILTER)
  const hasSources = sources.length > 0
  return (
    <div className="@container flex flex-wrap items-center gap-2">
      <div className="flex shrink-0 items-center gap-2">
        <DateRangeChip align="start" />
      </div>
      <div className="flex w-full min-w-0 flex-wrap items-center gap-2 @[60rem]:w-auto">
        {hasSources ? <SourceChip bar /> : null}
        <ModelChip bar />
        <DeviceScopeControl bar />
      </div>
    </div>
  )
}
