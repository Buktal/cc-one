// The overview's window snapshot (#106): stats + trend reads under one
// filter plus the delta / average derivations, shared by the overview's two
// cards (TokenHero's headline and the KPI band's 日均 Token / delta sub).
// Extracted from TokenHero so the caliber rules — multi-day delta = last day
// vs window start, single-day delta = today vs yesterday's same hours —
// exist once. All reads cache under the same filterId, so both cards share
// one fetch.

import dayjs from "dayjs"
import { useMemo } from "react"

import { useStatsQuery, useTrendQuery, ZERO_STATS } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { hourlySnapshot, tokenSnapshot } from "@/features/usage/derive"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import type { TrendBucket } from "@/types/generated/bindings"

export interface TokenSnapshotView {
  stats: NonNullable<Awaited<ReturnType<typeof useStatsQuery>>["data"]>
  /** Window delta ratio: multi-day = last vs first day, single-day = today vs
   *  yesterday's same hours. null when no comparison base exists. */
  deltaPct: number | null
  /** Single-day window (hourly trend resolution). */
  singleDay: boolean
  /** Multi-day daily token average (total / window days); 0 on single-day. */
  dailyAvg: number
  /** Single-day hourly average; 0 on multi-day. */
  hourlyAvg: number
}

export function useTokenSnapshot(filter: FilterState): TokenSnapshotView {
  const { data: stats } = useStatsQuery(filter)
  // 单日窗口 (today / 单日 custom) 判定与趋势图一致: 时间戳范围落在同一个
  // 本地日 → 小时粒度。此时多日的 delta/日均语义失真 (日趋势只有 1 个点,
  // delta 恒 null, 日均退化回总量), 改走 hourlySnapshot 的「vs 昨日同时段」。
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  const singleDay = !!fromTs && !!toTs && dayjs(fromTs).isSame(toTs, "day")
  // 昨日 filter —— 与当前 filter 同维度 (model/source/device), 只把窗口换成
  // 昨天的具体日期。queryKey 一天内稳定, 跨天滚动自动重查。
  const yesterdayFilter = useMemo<FilterState>(
    () =>
      singleDay
        ? {
            ...filter,
            range_preset: "custom",
            from_day: dayjs(from_day).subtract(1, "day").format("YYYY-MM-DD"),
            to_day: dayjs(from_day).subtract(1, "day").format("YYYY-MM-DD"),
          }
        : filter,
    [filter, singleDay, from_day],
  )
  const bucket: TrendBucket = singleDay ? "Hour" : "Day"
  const { data: trend = [] } = useTrendQuery({ filter, bucket })
  const { data: yesterday = [] } = useTrendQuery(
    { filter: yesterdayFilter, bucket: "Hour" },
    { skip: !singleDay },
  )
  const s = stats ?? ZERO_STATS
  const now = dayjs()
  const hourlySnap = singleDay ? hourlySnapshot(trend, yesterday, now) : null
  const multiSnap = singleDay ? null : tokenSnapshot(s, trend)
  return {
    stats: s,
    deltaPct: hourlySnap?.deltaPct ?? multiSnap?.deltaPct ?? null,
    singleDay,
    dailyAvg: multiSnap?.dailyAvg ?? 0,
    hourlyAvg: hourlySnap?.hourlyAvg ?? 0,
  }
}
