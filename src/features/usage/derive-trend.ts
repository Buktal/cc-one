// 趋势派生（usage 域 derive 的 trend 拆分）：小时补零（zeroFillTrend）与
// 堆叠构成的百分比视图（shareStackTrend）。前者把单日窗口的 x 轴铺满本地
// 日（后端 GROUP BY 只产出有记录的桶）；后者为趋势图的「占比」模式把每点
// 四桶换成「占可见桶总量的比率」——分母随图例显隐收窄：隐藏一桶 = 它不
// 参与构成，而不是被算成 0，剩余桶的占比始终归一。纯函数、`now` 注入。

import type { Dayjs } from "dayjs"

import {
  BUCKET_DISPLAY,
  type BucketStatKey,
  sumBuckets,
  type TokenBucketKey,
  tokenBuckets,
} from "@/lib/token-buckets"
import type { TrendPoint } from "@/types/generated/bindings"

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

/** 百分比堆叠视图的点形：`*_tokens` 是 0–1 的占比（dataKey 与绝对视图
 *  同名，堆叠面积换一份数据即切模式）；`*_tokens_abs` 保留绝对值供
 *  tooltip 拼「占比 · 绝对量」；total_tokens 是四桶绝对总量（合计行）。
 *  request_count 原样透传——点形保持 TrendPoint 的结构超集，recharts 的
 *  data 槽位两种点形通吃。 */
export interface ShareTrendPoint {
  day: string
  input_tokens: number
  output_tokens: number
  cache_creation_tokens: number
  cache_read_tokens: number
  input_tokens_abs: number
  output_tokens_abs: number
  cache_creation_tokens_abs: number
  cache_read_tokens_abs: number
  total_tokens: number
  total_cost_usd: number | null
  request_count: number
}

/** 每点四桶 → 占可见桶总量的比率（0–1）。`visible` 缺省 = 名册全桶；
 *  分母为 0 的点（无数据或可见桶全空）各桶比率归 0，不除零。 */
export function shareStackTrend(
  points: readonly TrendPoint[],
  visible?: ReadonlySet<BucketStatKey>,
): ShareTrendPoint[] {
  const vis =
    visible ??
    new Set<BucketStatKey>(
      BUCKET_DISPLAY.map((b): BucketStatKey => `${b.bucket}_tokens`),
    )
  return points.map((p) => {
    const buckets = tokenBuckets(p)
    const denom = BUCKET_DISPLAY.reduce(
      (s, b) => (vis.has(`${b.bucket}_tokens`) ? s + buckets[b.bucket] : s),
      0,
    )
    const pct = (k: TokenBucketKey): number =>
      denom > 0 ? buckets[k] / denom : 0
    return {
      day: p.day,
      input_tokens: pct("input"),
      output_tokens: pct("output"),
      cache_creation_tokens: pct("cache_creation"),
      cache_read_tokens: pct("cache_read"),
      input_tokens_abs: buckets.input,
      output_tokens_abs: buckets.output,
      cache_creation_tokens_abs: buckets.cache_creation,
      cache_read_tokens_abs: buckets.cache_read,
      total_tokens: sumBuckets(buckets),
      total_cost_usd: p.total_cost_usd,
      request_count: p.request_count,
    }
  })
}
