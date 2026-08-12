// Pure read-model derivations for the usage dashboard: trend zero-fill, token
// snapshot (multi-day delta vs window start + daily average, single-day delta
// vs yesterday same hours + hourly average), top-N model aggregation, and
// stop_reason → tone classification. Every function here is pure — `now` is
// injected and i18n labels are applied by the caller — so each is testable
// through its signature alone.

import type { Dayjs } from "dayjs"

import type {
  ModelStatsRow,
  TrendPoint,
  UsageStats,
} from "@/types/generated/bindings"

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

/**
 * stop_reason → semantic tone. Free-form source string matched by exact value
 * / contains. Color signals outcome: normal completion / tool call stay calm,
 * hitting a limit warns, a refusal / error alarms. Unknown or empty values
 * fall back to null (no chip).
 */
export function stopReasonTone(value: string): StopReasonTone {
  const v = value.toLowerCase()
  if (!v) return null
  if (v === "end_turn") return "success"
  if (v.includes("tool_use")) return "tool"
  if (
    v.includes("max_tokens") ||
    v.includes("exceeded") ||
    v.includes("context_window")
  )
    return "warn"
  if (v.includes("refusal") || v.includes("error")) return "error"
  return null
}
