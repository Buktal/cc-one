// Pure time-range derivations shared by the usage filter slice, the sessions
// browser hook, and the shared DateRangeChip component. Slices own the filter
// STATE; this file owns the math — what day bounds a preset means, and what a
// stored filter effectively means right now. Lives in lib/ so the shared
// component and both features depend on one neutral layer, never on an app
// slice.

import dayjs from "dayjs"

/**
 * Time-range preset. The dynamic ones (today / 7d / 30d / 1y) are the
 * source of truth — their day bounds are recomputed on every query, so "today"
 * stays today even across midnight (a dynamic preset never stores a concrete
 * date). "all" means no bounds; "custom" keeps the user-picked from_day /
 * to_day verbatim.
 */
export type Preset = "today" | "7d" | "30d" | "1y" | "all" | "custom"

/** The time-range half of a filter state: preset + stored day bounds. */
export interface DayRange {
  range_preset: Preset
  from_day: string
  to_day: string
}

/** Today's local date offset by `offset` days, as "YYYY-MM-DD". */
export function dayStr(offset = 0): string {
  return dayjs().add(offset, "day").format("YYYY-MM-DD")
}

/** Concrete day bounds for a dynamic preset. "custom" / "all" return empty —
 *  "custom" uses the user-picked from_day / to_day, "all" means no bounds. */
export function presetDays(p: Preset): Pick<DayRange, "from_day" | "to_day"> {
  switch (p) {
    case "today":
      return { from_day: dayStr(), to_day: dayStr() }
    case "7d":
      return { from_day: dayStr(-6), to_day: dayStr() }
    case "30d":
      return { from_day: dayStr(-29), to_day: dayStr() }
    case "1y":
      // 364 天回看 = 含今天的 365 天窗口（贡献图「近一年」档的唯一前置，
      // 也随三页共享对所有时间形态读生效）。
      return { from_day: dayStr(-364), to_day: dayStr() }
    default:
      return { from_day: "", to_day: "" }
  }
}

/** The EFFECTIVE day bounds for a filter: a dynamic preset (today / 7d /
 *  30d / 1y) is recomputed on the spot (it stores no concrete date), so it
 *  always means the current day window at query time regardless of when it
 *  was picked. "all" / "custom" return the stored values verbatim — "all"
 *  stores empty bounds, "custom" keeps the user-picked days. Single place
 *  that answers "what days does this filter mean", shared by the endpoint
 *  queryFns and the DateRangeChip display. */
export function effectiveDays(
  f: Pick<DayRange, "range_preset" | "from_day" | "to_day">,
): Pick<DayRange, "from_day" | "to_day"> {
  if (
    f.range_preset === "today" ||
    f.range_preset === "7d" ||
    f.range_preset === "30d" ||
    f.range_preset === "1y"
  ) {
    return presetDays(f.range_preset)
  }
  return { from_day: f.from_day, to_day: f.to_day }
}

/** Local-day range → inclusive UTC ISO8601 timestamp bounds. The backend
 *  filters on a UTC `timestamp` (not the UTC `day` bucket): a local "today" in
 *  UTC+8 straddles two UTC days, so we widen to timestamps or the early-morning
 *  rows (whose UTC day is still yesterday) vanish from "today". Empty day →
 *  null (no bound). Shared by the usage (toFilter) and sessions
 *  (buildSessionFilter) query paths so the local-day → UTC widening lives in
 *  one place. */
export function dayRangeToTs(
  from_day: string,
  to_day: string,
): { from_ts: string | null; to_ts: string | null } {
  return {
    from_ts: from_day ? dayjs(from_day).startOf("day").toISOString() : null,
    to_ts: to_day ? dayjs(to_day).endOf("day").toISOString() : null,
  }
}

/** 单日窗口判定：时间戳范围落在同一个本地日 → 趋势按小时分桶，否则按天。
 *  request-section / usage-trend-chart / use-token-snapshot 三处共用此一谓词
 *  （曾各抄一遍 isSame("day")、注释互指）。跨时区语义：UTC+8 的「今天」映射成
 *  仍属同一本地日的 24h UTC 窗口，isSame("day") 在本地日上判定即正确命中。 */
export function sameDayWindow(
  fromTs?: string | null,
  toTs?: string | null,
): boolean {
  return !!fromTs && !!toTs && dayjs(fromTs).isSame(toTs, "day")
}
