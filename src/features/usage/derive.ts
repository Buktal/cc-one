// Pure read-model derivations for the usage dashboard: trend zero-fill, token
// snapshot (multi-day delta vs window start + daily average, single-day delta
// vs yesterday same hours + hourly average), top-N model aggregation, and
// stop_reason → tone classification. Every function here is pure — `now` is
// injected and i18n labels are applied by the caller — so each is testable
// through its signature alone.

import dayjs, { type Dayjs } from "dayjs"

import {
  FILTER_DIMENSIONS,
  type FilterState,
} from "@/app/store/slices/filterSlice"
import { type FilterOption, facetOptions } from "@/lib/filter-options"
import { projectBasename } from "@/lib/paths"
import type {
  ModelStatsRow,
  TrendPoint,
  UsageStats,
} from "@/types/generated/bindings"

/** Stable cache id for a FilterState (so each filter scope caches
 *  independently). Built from the logical dimensions only — a dynamic preset
 *  stores no date, so the id stays stable across a day and the bounds roll via
 *  the collect-interval refresh chain. Concatenates every FilterState
 *  dimension (FILTER_DIMENSIONS) — derive.test.ts fails if one is missed, so
 *  differing dimension values can never silently share a cache entry. */
export function filterId(f: FilterState): string {
  return FILTER_DIMENSIONS.map((k) => f[k]).join("|")
}

/**
 * Project-dropdown options from the distinct-projects candidates. Known
 * identities show their basename (the tree / table convention — full paths
 * live on hover there and stay out of the dropdown); the unknown sentinel
 * shows the labeled special option. `unknownOption` is the LIVE presence
 * probe (the option is offered only while the endpoint reports unknown usage
 * in the window), while `unknownValue` is the stable value (remembered after
 * first sight) used for LABELING — a selected sentinel that a window change
 * dropped from the candidates still merges back via facetOptions and must
 * still read as「未知项目」, not as its raw literal. A stale known project
 * merges back the same way and keeps its basename label.
 */
export function projectOptions(
  projects: readonly string[],
  unknownOption: string | null,
  unknownValue: string | null,
  selected: string,
  unknownLabel: string,
): FilterOption[] {
  const candidates = unknownOption
    ? facetOptions([...projects, unknownOption], selected)
    : facetOptions(projects, selected)
  return candidates.map((v) => ({
    value: v,
    label:
      unknownValue != null && v === unknownValue
        ? unknownLabel
        : projectBasename(v),
  }))
}

/** A zero-valued trend point used to pad empty hour buckets. */
export function zeroTrendPoint(day: string): TrendPoint {
  return {
    day,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    total_tokens: 0,
    total_cost_usd: 0,
  }
}

/**
 * Pad an hourly trend so the x-axis spans the selected local day in full, not
 * only the hours that happen to have records (the backend GROUP BY omits empty
 * buckets). For the current day the axis stops at `now`'s hour (no future
 * buckets); for a past day it runs 00:00 → 23:00 so the whole day is visible.
 * `target` (the selected day) and `now` (the clock) are both injected — never
 * read inside — so the output is deterministic and testable. An empty input is
 * returned unchanged so the caller keeps its empty state. Only call this for
 * an hourly (single-day) range; a multi-day range is the backend's per-day
 * buckets as-is.
 */
export function zeroFillTrend(
  rawData: TrendPoint[],
  target: Dayjs,
  now: Dayjs,
): TrendPoint[] {
  if (rawData.length === 0) return rawData
  const byKey = new Map(rawData.map((p) => [p.day, p]))
  const out: TrendPoint[] = []
  const end = target.isSame(now, "day") ? now : target.endOf("day")
  let cur = target.startOf("day")
  while (!cur.isAfter(end, "hour")) {
    const key = cur.format("YYYY-MM-DDTHH")
    out.push(byKey.get(key) ?? zeroTrendPoint(key))
    cur = cur.add(1, "hour")
  }
  return out
}

export interface TokenSnapshot {
  /** Last-day vs window-start ratio, or null (< 2 points or zero start). */
  deltaPct: number | null
  /** Total tokens divided by the number of days in the window. */
  dailyAvg: number
}

/**
 * Window snapshot from stats + per-day trend: delta = last day vs first day
 * (trend is day-ascending), daily average = total tokens / day count.
 */
export function tokenSnapshot(
  stats: UsageStats,
  trend: TrendPoint[],
): TokenSnapshot {
  const points = trend.map((p) => Number(p.total_tokens ?? 0))
  let deltaPct: number | null = null
  if (points.length >= 2) {
    const first = points[0]
    const last = points[points.length - 1]
    if (first > 0) deltaPct = (last - first) / first
  }
  const dailyAvg = points.length > 0 ? stats.total_tokens / points.length : 0
  return { deltaPct, dailyAvg }
}

export interface HourlySnapshot {
  /** 今日已发生 vs 昨日同时段比值, 或 null (昨日无数据)。 */
  deltaPct: number | null
  /** 今日总量 / 已过小时数 —— 单日窗口下「日均」的等价物。 */
  hourlyAvg: number
}

/**
 * 单日 (hourly) 窗口的对比快照: 今日 [0 点, now] 各小时之和 vs 昨日相同时段
 * 各小时之和, 以及每小时均值。`now` 注入保证可测。
 *
 * 为什么存在: 多日窗口的 tokenSnapshot (末点 vs 首点 + 日均) 在单日窗口下
 * 语义失真 —— 只有一天时趋势点只剩 1 个, delta 恒为 null, 「日均」退化为
 * 总量本身。这里改为「vs 昨日同时段」, 涨幅才是用户能读懂的「今天比昨天
 * 快/慢了」; 昨日全天无数据 (未采集) 时 delta 为 null, 调用方不渲染涨幅。
 */
export function hourlySnapshot(
  today: TrendPoint[],
  yesterday: TrendPoint[],
  now: Dayjs,
): HourlySnapshot {
  const hourSum = (points: TrendPoint[], day: string, h: number) =>
    Number(
      points.find((p) => p.day === `${day}T${String(h).padStart(2, "0")}`)
        ?.total_tokens ?? 0,
    )
  const todayPrefix = now.format("YYYY-MM-DD")
  const yesterdayPrefix = now.subtract(1, "day").format("YYYY-MM-DD")
  const cutoff = now.hour()
  let todaySum = 0
  let yesterdaySum = 0
  for (let h = 0; h <= cutoff; h++) {
    todaySum += hourSum(today, todayPrefix, h)
    yesterdaySum += hourSum(yesterday, yesterdayPrefix, h)
  }
  const deltaPct =
    yesterdaySum > 0 ? (todaySum - yesterdaySum) / yesterdaySum : null
  return { deltaPct, hourlyAvg: todaySum / (cutoff + 1) }
}

export type ModelMetric = "cost" | "tokens"

/** Numeric value of a model row under the chosen metric. */
export function modelMetricValue(
  row: ModelStatsRow,
  metric: ModelMetric,
): number {
  return metric === "cost"
    ? Number(row.total_cost_usd ?? 0)
    : Number(row.total_tokens)
}

export interface TopNResult {
  top: { model: string; value: number; cache_hit_rate: number }[]
  rest: { count: number; sum: number }
  /** Sum of all rows (>= 1, so callers can divide safely). */
  total: number
}

/** Top-N models by metric plus an aggregate of the remainder. Pure — the
 *  caller renders the "others" label via i18n from `rest.count`. The cache
 *  hit rate is carried through from the backend (single implementation);
 *  the "others" aggregate has none, callers render it conditionally. */
export function topNModels(
  rows: ModelStatsRow[],
  metric: ModelMetric,
  topN: number,
): TopNResult {
  const sorted = [...rows].sort(
    (a, b) => modelMetricValue(b, metric) - modelMetricValue(a, metric),
  )
  const topRows = sorted.slice(0, topN)
  const rest = sorted.slice(topN)
  const restSum = rest.reduce((sum, r) => sum + modelMetricValue(r, metric), 0)
  const total =
    sorted.reduce((sum, r) => sum + modelMetricValue(r, metric), 0) || 1
  return {
    top: topRows.map((r) => ({
      model: r.model,
      value: modelMetricValue(r, metric),
      cache_hit_rate: Number(r.cache_hit_rate ?? 0),
    })),
    rest: { count: rest.length, sum: restSum },
    total,
  }
}

export type StopReasonTone = "success" | "tool" | "warn" | "error" | null

/** stop_reason exact/contains match → i18n label key for the chip text.
 *  Keys resolve under `usage.logs.stopReason.*`; the English raw value stays
 *  in the chip's title tooltip. Order matters for contains-matching compound
 *  values like `context_window_exceeded` — the first pattern that matches wins
 *  the label, while the tone follows the same branch the old matcher took. */
const STOP_REASON_LABELS: [string, string][] = [
  ["end_turn", "usage.logs.stopReason.endTurn"],
  ["tool_use", "usage.logs.stopReason.toolUse"],
  ["max_tokens", "usage.logs.stopReason.maxTokens"],
  ["context_window", "usage.logs.stopReason.contextWindow"],
  ["exceeded", "usage.logs.stopReason.exceeded"],
  ["refusal", "usage.logs.stopReason.refusal"],
  ["error", "usage.logs.stopReason.error"],
]

/** tone for a matched pattern — mirrors the original stopReasonTone branches
 *  so tone and label never disagree about which bucket a value lands in. */
function toneFor(pattern: string): StopReasonTone {
  if (pattern === "end_turn") return "success"
  if (pattern === "tool_use") return "tool"
  if (
    pattern === "max_tokens" ||
    pattern === "exceeded" ||
    pattern === "context_window"
  )
    return "warn"
  if (pattern === "refusal" || pattern === "error") return "error"
  return null
}

export interface StopReasonInfo {
  tone: StopReasonTone
  /** i18n label key, or null when the value is unknown (render raw). */
  labelKey: string | null
}

/**
 * stop_reason → { tone, labelKey } in one pass — the single classifier shared
 * by `stopReasonTone` (chip color) and `stopReasonLabelKey` (chip text) so the
 * two never drift apart. Free-form source string matched by exact value first
 * (specific labels), then by contains (compound values). Unknown or empty
 * values fall back to null / null (no chip).
 */
export function classifyStopReason(value: string): StopReasonInfo {
  const v = value.toLowerCase()
  if (!v) return { tone: null, labelKey: null }
  for (const [pattern, labelKey] of STOP_REASON_LABELS) {
    if (v === pattern) return { tone: toneFor(pattern), labelKey }
  }
  for (const [pattern, labelKey] of STOP_REASON_LABELS) {
    if (v.includes(pattern)) return { tone: toneFor(pattern), labelKey }
  }
  return { tone: null, labelKey: null }
}

/** stop_reason → semantic tone. Color signals outcome: normal completion /
 *  tool call stay calm, hitting a limit warns, a refusal / error alarms. */
export function stopReasonTone(value: string): StopReasonTone {
  return classifyStopReason(value).tone
}

/** stop_reason → i18n label key for the chip text, or null when unknown. */
export function stopReasonLabelKey(value: string): string | null {
  return classifyStopReason(value).labelKey
}

/** A single call is "notable" when it costs at least this much (USD) — the
 *  log table highlights such rows so expensive calls surface at a glance. */
export const COST_NOTABLE_THRESHOLD = 1

/** Whether a call cost is notable enough to highlight (null/absent → false). */
export function costIsNotable(usd: number | null | undefined): boolean {
  return Number(usd ?? 0) >= COST_NOTABLE_THRESHOLD
}

export interface DayGroup<T> {
  /** Local `YYYY-MM-DD` of the rows in this group (caller formats for display). */
  dayKey: string
  rows: T[]
}

/** Split time-desc rows into consecutive local-day groups so the ledger can
 *  insert a day separator when the window spans multiple days. Rows are
 *  assumed time-desc (as the log query returns); a day boundary is a new
 *  group. Unparseable timestamps fall back to their raw `yyyy-mm-dd` prefix. */
export function groupRowsByDay<T extends { timestamp: string }>(
  rows: T[],
): DayGroup<T>[] {
  const groups: DayGroup<T>[] = []
  for (const r of rows) {
    const d = dayjs(r.timestamp)
    const dayKey = d.isValid()
      ? d.format("YYYY-MM-DD")
      : r.timestamp.slice(0, 10)
    const last = groups[groups.length - 1]
    if (last && last.dayKey === dayKey) last.rows.push(r)
    else groups.push({ dayKey, rows: [r] })
  }
  return groups
}
