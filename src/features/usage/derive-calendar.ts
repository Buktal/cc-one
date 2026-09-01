// 日历热力的格子派生（纯函数层，与 calendar-heatmap 同名同域）：GitHub 式
// 周历的按天格子（calendarCells）与短窗「天 × 小时」矩阵行（hourMatrixRows）。
// 纯函数、不注入时钟——窗口完全由调用方喂进来的点列决定，图形不自带统计
// 窗口。

import dayjs from "dayjs"

import type { TrendPoint } from "@/types/generated/bindings"

/** One calendar-heatmap cell (GitHub contribution-graph shape): a week-column
 *  × weekday-row grid over the filter window's Day buckets. */
export interface CalendarCell {
  /** Local ISO day `YYYY-MM-DD` (the TrendPoint Day bucket key). */
  day: string
  /** Day total tokens — drives the color level. */
  tokens: number
  requests: number
  cost: number
  /** Week column (0-based; the grid is Monday-first). */
  col: number
  /** Weekday row: Monday = 0 … Sunday = 6. */
  row: number
  /** 0 = no usage; 1–4 = quartile of the non-zero days (GitHub
   *  ContributionLevel 口径: quantile steps, not absolute-value linear). */
  level: 0 | 1 | 2 | 3 | 4
}

/** 热力色阶的档位切点：非零值的四分位（q1/q2/q3）。绝对值线性会被重度
 *  使用者的量级压成两档可读，分位档任何量级都保持五档可辨；无非零值时
 *  返回的切点永远不会被命中（全零窗 → 全 NONE）。 */
export function quartileThresholds(
  values: readonly number[],
): readonly [number, number, number] {
  const nz = [...values].filter((v) => v > 0).sort((a, b) => a - b)
  if (nz.length === 0) return [1, 2, 3]
  // q(p) picks the sorted non-zero value at floor(p·n).
  const q = (p: number) =>
    nz[Math.min(nz.length - 1, Math.floor(p * nz.length))]
  return [q(0.25), q(0.5), q(0.75)]
}

/** 值 → 色阶 0–4（0 = 无用量 NONE，1–4 = 对切点的四档分位）。 */
export function quartileLevel(
  v: number,
  thresholds: readonly [number, number, number],
): 0 | 1 | 2 | 3 | 4 {
  const [q1, q2, q3] = thresholds
  return v <= 0 ? 0 : v <= q1 ? 1 : v <= q2 ? 2 : v <= q3 ? 3 : 4
}

/**
 * Day-bucket trend → calendar cells. 输入契约：Day 桶、按天升序（后端
 * GROUP BY 的既定序）——首格的星期决定首列下移量，乱序会静默打乱网格，
 * 故不做防御性排序。色阶按「非零日的四分位」切 NONE + 四档。
 * 窄窗口自然退化（几天 = 几格），空窗口返回空（调用方渲染空态）。
 */
export function calendarCells(points: readonly TrendPoint[]): CalendarCell[] {
  if (points.length === 0) return []
  // dayjs .day(): 0 = Sunday … 6 = Saturday → Monday-first row index.
  const lead = (dayjs(points[0].day).day() + 6) % 7
  const thresholds = quartileThresholds(
    points.map((p) => Number(p.total_tokens)),
  )
  return points.map((p, i) => {
    const row = (lead + i) % 7
    return {
      day: p.day,
      tokens: Number(p.total_tokens),
      requests: Number(p.request_count),
      cost: Number(p.total_cost_usd ?? 0),
      col: Math.floor((lead + i) / 7),
      row,
      level: quartileLevel(Number(p.total_tokens), thresholds),
    }
  })
}

/** One hour-matrix cell: a single local hour of one day. */
export interface HourMatrixCell {
  /** 0–23, parsed from the Hour bucket key `YYYY-MM-DDTHH`. */
  hour: number
  tokens: number
  requests: number
  cost: number
  /** Same quartile scale as CalendarCell.level, over the window's hours. */
  level: 0 | 1 | 2 | 3 | 4
}

/** One matrix row = one local day, hours in ascending order. */
export interface HourMatrixRow {
  /** Local ISO day `YYYY-MM-DD`. */
  day: string
  cells: HourMatrixCell[]
}

/**
 * Hour-bucket trend → hour-matrix rows (the short-window calendar: row = day,
 * column = hour 0–23). 输入契约：Hour 桶、升序、已铺满窗口（zeroFillTrend
 * 的输出——空小时是显式零点，不补就会把矩阵列位挤错）；分组按桶键的日期
 * 前缀，行内自然保持小时升序。色阶 = 非零小时的四分位（与周历同一档位
 * 语义，切点各自按本窗口算）。空窗口返回空（调用方渲染空态）。
 */
export function hourMatrixRows(points: readonly TrendPoint[]): HourMatrixRow[] {
  if (points.length === 0) return []
  const thresholds = quartileThresholds(
    points.map((p) => Number(p.total_tokens)),
  )
  const rows: HourMatrixRow[] = []
  let cur: HourMatrixRow | null = null
  for (const p of points) {
    const day = p.day.slice(0, 10)
    if (!cur || cur.day !== day) {
      cur = { day, cells: [] }
      rows.push(cur)
    }
    cur.cells.push({
      hour: Number(p.day.slice(11, 13)),
      tokens: Number(p.total_tokens),
      requests: Number(p.request_count),
      cost: Number(p.total_cost_usd ?? 0),
      level: quartileLevel(Number(p.total_tokens), thresholds),
    })
  }
  return rows
}
