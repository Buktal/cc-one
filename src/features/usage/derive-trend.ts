// 趋势派生（usage 域 derive 的 trend 拆分）：桶铺满（zeroFillTrend /
// zeroFillDailyTrend——把 x 轴/矩阵铺满窗口而非只铺有记录的桶，后端
// GROUP BY 略过空桶；空位不补，日历格会挤错星期位）、堆叠构成的百分比
// 视图（shareStackTrend）与累计爬坡（cumulativeTrend）。
// 后者为趋势图的「占比」模式把每点四桶换成「占可见桶总量的比率」——分母
// 随图例显隐收窄：隐藏一桶 = 它不参与构成，而不是被算成 0，剩余桶的占比
// 始终归一。纯函数、`now` 注入。

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
 * Pad a bucketed trend so the axis spans the window `[from, to]` in full, not
 * only the buckets that happen to have records (the backend GROUP BY omits
 * empty buckets). `from` / `to` / `now` are all injected — never read inside —
 * so the output is deterministic and testable. An empty input is returned
 * unchanged so the caller keeps its empty state.
 */
function zeroFillBuckets(
  rawData: TrendPoint[],
  from: Dayjs,
  to: Dayjs,
  now: Dayjs,
  step: "hour" | "day",
  keyOf: (d: Dayjs) => string,
): TrendPoint[] {
  if (rawData.length === 0) return rawData
  const byKey = new Map(rawData.map((p) => [p.day, p]))
  const out: TrendPoint[] = []
  // 小时粒度的「今天」截到当前小时（未来小时还没发生，不产桶）；天粒度
  // 的今天本身就是一格完整天（当天已发生的累计），不截。
  const end = step === "hour" && to.isSame(now, "day") ? now : to
  let cur = from.startOf(step)
  const stop = end.startOf(step)
  while (!cur.isAfter(stop, step)) {
    const key = keyOf(cur)
    out.push(byKey.get(key) ?? zeroTrendPoint(key))
    cur = cur.add(1, step)
  }
  return out
}

/**
 * Pad an hourly trend so the axis spans the window `[from, to]` in full.
 * Single-day windows (the trend chart) and short multi-day windows (the
 * calendar's hour matrix, ≤ 7 days) share this one implementation. When `to`
 * lands on today the axis stops at `now`'s hour (no future buckets); otherwise
 * it runs to the window's last hour.
 */
export function zeroFillTrend(
  rawData: TrendPoint[],
  from: Dayjs,
  to: Dayjs,
  now: Dayjs,
): TrendPoint[] {
  return zeroFillBuckets(rawData, from, to, now, "hour", (d) =>
    d.format("YYYY-MM-DDTHH"),
  )
}

/**
 * Pad a daily trend so the axis spans the window `[from, to]` in full. The
 * calendar's month/weekgrid forms need it: grid cells are placed by calendar
 * position (weekday rows/columns), so a missing zero day would shift every
 * later record into the wrong slot. `to` landing on today stays a full day —
 * the day bucket is the accumulated-so-far total, one cell.
 */
export function zeroFillDailyTrend(
  rawData: TrendPoint[],
  from: Dayjs,
  to: Dayjs,
  now: Dayjs,
): TrendPoint[] {
  return zeroFillBuckets(rawData, from, to, now, "day", (d) =>
    d.format("YYYY-MM-DD"),
  )
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

/** 累计视图的点形：TrendPoint 的结构超集 + cum（窗口首点至今的 token
 *  前缀和）——与 ShareTrendPoint 同思路，recharts 的 data 槽位对绝对/
 *  占比/累计三种点形通吃；当日增量直接读 total_tokens（同字段同义）。 */
export type CumTrendPoint = TrendPoint & { cum: number }

/** 逐点 token 前缀和——趋势图「累计」模式的数据（单调爬坡，斜率即消耗
 *  加速度）。Day/Hour 桶通吃（单日窗口逐小时累计同样成立）。 */
export function cumulativeTrend(
  points: readonly TrendPoint[],
): CumTrendPoint[] {
  let cum = 0
  return points.map((p) => {
    cum += Number(p.total_tokens ?? 0)
    return { ...p, cum }
  })
}
