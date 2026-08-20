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
  ProjectUsageRow,
  SessionUsageRow,
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
    request_count: 0,
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

// ------------------------------------------------- dashboard sections (#106) ----

export interface ProjectRankingRest {
  count: number
  tokens: number
  sessions: number
  requests: number
  cost: number
}

export interface ProjectRanking {
  /** Top `topN` known projects, tokens-desc (backend order preserved). */
  top: ProjectUsageRow[]
  /** Aggregate over the known projects beyond `topN`, or null (none). */
  rest: ProjectRankingRest | null
  /** The unknown bucket row (`is_unknown`), or null when the window has none. */
  unknown: ProjectUsageRow | null
  /** Known-project count (excluding the unknown bucket). */
  knownCount: number
  /** All-bucket token sum, floored at 1 so callers can divide safely. */
  totalTokens: number
  /** Top-3 known projects' share of ALL bucket tokens, or null (< 3 known). */
  top3Share: number | null
}

/** Split usage-grain project buckets into the ranking card's parts: top-N
 *  known rows, an "others" aggregate, and the unknown row on its own (it
 *  renders hatched like `rest` but keeps its own label / click-to-filter
 *  value). The unknown bucket never enters `top` even when it outranks a
 *  known project — it is a data-quality bucket, not a leaderboard entry. */
export function projectRanking(
  rows: readonly ProjectUsageRow[],
  topN: number,
): ProjectRanking {
  const unknown = rows.find((r) => r.is_unknown) ?? null
  const known = rows.filter((r) => !r.is_unknown)
  const top = known.slice(0, topN)
  const others = known.slice(topN)
  const totalTokens =
    rows.reduce((sum, r) => sum + Number(r.total_tokens), 0) || 1
  const rest: ProjectRankingRest | null =
    others.length > 0
      ? {
          count: others.length,
          tokens: others.reduce((s, r) => s + Number(r.total_tokens), 0),
          sessions: others.reduce((s, r) => s + Number(r.session_count), 0),
          requests: others.reduce((s, r) => s + Number(r.request_count), 0),
          cost: others.reduce((s, r) => s + Number(r.total_cost_usd ?? 0), 0),
        }
      : null
  const top3Share =
    known.length >= 3
      ? known.slice(0, 3).reduce((s, r) => s + Number(r.total_tokens), 0) /
        totalTokens
      : null
  return {
    top,
    rest,
    unknown,
    knownCount: known.length,
    totalTokens,
    top3Share,
  }
}

export interface SessionTopRow {
  session_id: string
  device_id: string
  title: string
  tokens: number
  /** Share over the section total ([0,1]). */
  share: number
}

export interface SessionSectionStats {
  sessions: number
  /** Subagent sessions (non-empty agent_type) and their share. */
  subagents: number
  subagentShare: number | null
  /** Distinct devices among the rows. */
  devices: number
  /** Top device's token share, or null (no rows). */
  topDeviceShare: number | null
  /** Longest span (last_active − started, ms), or null (none positive). */
  longestSpanMs: number | null
  /** Total turns / sessions, or null (no sessions). */
  avgTurns: number | null
  /** Turn-count buckets by session: 1–3 / 4–8 / 9–16 / 17+. */
  turnBuckets: [number, number, number, number]
  /** Top `topN` sessions by tokens (backend order preserved). */
  top: SessionTopRow[]
  /** All-row token sum, floored at 1 so callers can divide safely. */
  totalTokens: number
}

/** The session section's aggregates over usage-grain session rows: counts /
 *  shares / spans / the 1–3·4–8·9–16·17+ turn buckets / the top-N list. Pure
 *  so the 口径 invariants (shares divide by the SAME total the list renders,
 *  buckets partition the session set) are testable. */
export function sessionSectionStats(
  rows: readonly SessionUsageRow[],
  topN: number,
): SessionSectionStats {
  const totalTokens =
    rows.reduce(
      (s, r) =>
        s +
        Number(r.input_tokens) +
        Number(r.output_tokens) +
        Number(r.cache_creation_tokens) +
        Number(r.cache_read_tokens),
      0,
    ) || 1
  const turnBuckets: [number, number, number, number] = [0, 0, 0, 0]
  const byDevice = new Map<string, number>()
  let subagents = 0
  let turns = 0
  let longest: number | null = null
  for (const r of rows) {
    const tokens =
      Number(r.input_tokens) +
      Number(r.output_tokens) +
      Number(r.cache_creation_tokens) +
      Number(r.cache_read_tokens)
    byDevice.set(r.device_id, (byDevice.get(r.device_id) ?? 0) + tokens)
    if (r.agent_type) subagents += 1
    turns += Number(r.turn_count)
    const span = Date.parse(r.last_active_at) - Date.parse(r.started_at)
    if (Number.isFinite(span) && span > 0) {
      if (longest === null || span > longest) longest = span
    }
    turnBuckets[
      r.turn_count <= 3 ? 0 : r.turn_count <= 8 ? 1 : r.turn_count <= 16 ? 2 : 3
    ] += 1
  }
  const topDeviceTokens = byDevice.size > 0 ? Math.max(...byDevice.values()) : 0
  return {
    sessions: rows.length,
    subagents,
    subagentShare: rows.length > 0 ? subagents / rows.length : null,
    devices: byDevice.size,
    topDeviceShare: byDevice.size > 0 ? topDeviceTokens / totalTokens : null,
    longestSpanMs: longest,
    avgTurns: rows.length > 0 ? turns / rows.length : null,
    turnBuckets,
    top: rows.slice(0, topN).map((r) => ({
      session_id: r.session_id,
      device_id: r.device_id,
      title: r.title,
      tokens:
        Number(r.input_tokens) +
        Number(r.output_tokens) +
        Number(r.cache_creation_tokens) +
        Number(r.cache_read_tokens),
      share:
        (Number(r.input_tokens) +
          Number(r.output_tokens) +
          Number(r.cache_creation_tokens) +
          Number(r.cache_read_tokens)) /
        totalTokens,
    })),
    totalTokens,
  }
}

/** The requests section's headline figures from the per-bucket request counts
 *  (the trend's own bucket resolution — day, or hour on a single-day window):
 *  daily average and the peak bucket. Pure; empty input → nulls. */
export function requestHeadline(
  counts: readonly { day: string; request_count: number }[],
  windowDays: number | null,
): {
  dailyAvg: number | null
  peakCount: number | null
  peakDay: string | null
} {
  if (counts.length === 0)
    return { dailyAvg: null, peakCount: null, peakDay: null }
  const total = counts.reduce((s, p) => s + Number(p.request_count), 0)
  let peak = counts[0]
  for (const p of counts) if (p.request_count > peak.request_count) peak = p
  return {
    dailyAvg: windowDays && windowDays > 0 ? total / windowDays : null,
    peakCount: peak.request_count,
    peakDay: peak.day,
  }
}

/** Inclusive day count of an effective window, or null when unbounded ("all"
 *  stores no days — a daily average would be meaningless). */
export function windowDayCount(
  from_day: string,
  to_day: string,
): number | null {
  if (!from_day || !to_day) return null
  const diff = dayjs(to_day).diff(dayjs(from_day), "day")
  return Number.isFinite(diff) ? diff + 1 : null
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
