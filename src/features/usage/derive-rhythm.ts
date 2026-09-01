// 节律派生（#119「时间与节律」形态族的纯函数层，与看板该分区同名同域）：
// 日历热力的按天格子（calendarCells）。后续成员（时段热力矩阵、效率序列）
// 随各自的后端数据件落进本模块，不回 derive.ts（450/500 贴线）。纯函数、
// 不注入时钟——窗口完全由调用方喂进来的点列决定，图形不自带统计窗口。

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

/**
 * Day-bucket trend → calendar cells. 输入契约：Day 桶、按天升序（后端
 * GROUP BY 的既定序）——首格的星期决定首列下移量，乱序会静默打乱网格，
 * 故不做防御性排序。色阶按「非零日的四分位」切 NONE + 四档：重度使用者的
 * 绝对值会把线性色阶压成两档可读，分位档在任何量级下都保持五档可辨。
 * 窄窗口自然退化（几天 = 几格），空窗口返回空（调用方渲染空态）。
 */
export function calendarCells(points: readonly TrendPoint[]): CalendarCell[] {
  if (points.length === 0) return []
  // dayjs .day(): 0 = Sunday … 6 = Saturday → Monday-first row index.
  const lead = (dayjs(points[0].day).day() + 6) % 7
  // Non-zero-day quartiles: q(p) picks the sorted non-zero value at floor(p·n).
  const nz = points
    .map((p) => Number(p.total_tokens))
    .filter((v) => v > 0)
    .sort((a, b) => a - b)
  const q = (p: number) =>
    nz[Math.min(nz.length - 1, Math.floor(p * nz.length))]
  const [q1, q2, q3] = nz.length > 0 ? [q(0.25), q(0.5), q(0.75)] : [1, 2, 3]
  const level = (v: number): 0 | 1 | 2 | 3 | 4 =>
    v <= 0 ? 0 : v <= q1 ? 1 : v <= q2 ? 2 : v <= q3 ? 3 : 4
  return points.map((p, i) => {
    const row = (lead + i) % 7
    return {
      day: p.day,
      tokens: Number(p.total_tokens),
      requests: Number(p.request_count),
      cost: Number(p.total_cost_usd ?? 0),
      col: Math.floor((lead + i) / 7),
      row,
      level: level(Number(p.total_tokens)),
    }
  })
}
