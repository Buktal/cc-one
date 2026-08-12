// Shared usage-filter state. Lifted out of DashboardView so the
// toolbar's query conditions are shared across dashboard ⇆ logs. Empty
// string = "no constraint"; toFilter() converts to the nullable UsageFilter
// the API expects.
//
// The filter persists to localStorage so the chosen time range / model / device
// scope survive a restart (same pattern as the sidebar/control collapse flags).
// It is pure frontend query state — never sent to the Rust layer.

import { createSlice, type PayloadAction } from "@reduxjs/toolkit"
import dayjs from "dayjs"
import { useEffect, useMemo, useState } from "react"

import { useAppSelector } from "@/app/store/hooks"
import {
  type DayRange,
  dayStr,
  effectiveDays,
  isPreset,
  type Preset,
  presetDays,
} from "@/lib/date-range"
import type { UsageFilter } from "@/types/generated/bindings"

export const FILTER_STORAGE_KEY = "cc-one:usage-filter"

export interface FilterState extends DayRange {
  model: string
  source: string
  device_scope: string
}

export const EMPTY_FILTER: FilterState = {
  range_preset: "all",
  from_day: "",
  to_day: "",
  model: "",
  source: "",
  device_scope: "",
}

/** Read the persisted filter at startup; any shape mismatch ⇒ empty filter.
 *  Dynamic presets are re-expanded here so a restart never serves stale dates
 *  (the bug: "today" selected, then overnight it read back as "yesterday").
 *  Exported so the re-expand + legacy-back-compat behavior is unit-tested. */
export function loadPersistedFilter(): FilterState {
  const base = (over: Partial<FilterState>): FilterState => ({
    ...EMPTY_FILTER,
    ...over,
  })
  if (typeof localStorage === "undefined") return base({})
  const raw = localStorage.getItem(FILTER_STORAGE_KEY)
  if (!raw) return base({})
  try {
    const p = JSON.parse(raw) as Partial<FilterState>
    const str = (v: unknown) => (typeof v === "string" ? v : "")
    const common = {
      model: str(p.model),
      source: str(p.source),
      device_scope: str(p.device_scope),
    }
    // A missing preset is legacy data that stored only concrete dates — treat
    // it as "custom" so those literal bounds survive unchanged.
    const preset: Preset = isPreset(p.range_preset) ? p.range_preset : "custom"
    if (preset === "custom") {
      return base({
        range_preset: "custom",
        from_day: str(p.from_day),
        to_day: str(p.to_day),
        ...common,
      })
    }
    const days = presetDays(preset)
    return base({ range_preset: preset, ...days, ...common })
  } catch {
    return base({})
  }
}

/** Convert internal FilterState (empty = no constraint) → API UsageFilter (null). */
export function toFilter(s: FilterState): UsageFilter {
  const { from_day, to_day } = effectiveDays(s)
  return {
    // Local-day range → inclusive UTC timestamp bounds. The backend filters on
    // `timestamp` (UTC), not the UTC `day` bucket: a local "today" in UTC+8
    // straddles two UTC days, so we must widen to timestamps or the early-
    // morning rows (whose UTC day is still yesterday) vanish from "today".
    // effectiveDays keeps a dynamic preset rolling to the current day instead
    // of trusting the frozen stored dates (cross-midnight fix).
    from_ts: from_day ? dayjs(from_day).startOf("day").toISOString() : null,
    to_ts: to_day ? dayjs(to_day).endOf("day").toISOString() : null,
    model: s.model || null,
    source: s.source || null,
    device_scope: s.device_scope || null,
  }
}

/**
 * The active dashboard filter as an API-level UsageFilter (selector + toFilter).
 * Replaces the per-view `useMemo(() => toFilter(filter), [filter])` repetition —
 * the dashboard / logs / recent-requests views all bind the same filter, so the
 * selector + memo live once here.
 */
export function useUsageFilter(): UsageFilter {
  const filter = useAppSelector((s) => s.filter.filter)
  // Cross-midnight rollover: `dayStr()` in the memo deps only re-evaluates on
  // render, but an app left running overnight never re-renders on its own — so
  // the filter stayed frozen at yesterday and the cache kept serving stale data
  // past midnight. The 60s interval forces a render; setRolloverDay with the
  // same string is a no-op (React bails out), so only an actual day change
  // reaches the memo and re-queries the new day. The sessions browser uses the
  // same day-rollover tick (use-sessions-browser.ts).
  const [, setRolloverDay] = useState(dayStr)
  useEffect(() => {
    const id = setInterval(() => setRolloverDay(dayStr()), 60_000)
    return () => clearInterval(id)
  }, [])
  // `dayStr()` in the deps: the filter identity must flip at midnight so a
  // dynamic preset re-queries the new day's data. Dropping the memo entirely
  // would rebuild the object every render — request-log-table resets its page
  // offset on filter identity change, so that would clobber pagination.
  // biome-ignore lint/correctness/useExhaustiveDependencies: dayStr() is the cross-midnight rollover trigger — a stable value within a day, a new identity after midnight (that is the point)
  return useMemo(() => toFilter(filter), [filter, dayStr()])
}

/**
 * A "today only" UsageFilter scoped to one device — the lightweight card's
 * per-device today snapshot. `today` is passed in (not derived here) so the
 * caller controls when the day rolls over and tests can pin a date. Reuses
 * toFilter so the local-day → UTC bounds match the dashboard's "today" preset.
 */
export function todayFilter(deviceScope: string, today: string): UsageFilter {
  return toFilter({
    ...EMPTY_FILTER,
    from_day: today,
    to_day: today,
    device_scope: deviceScope,
  })
}

interface FilterSliceState {
  filter: FilterState
}

const initialState: FilterSliceState = { filter: loadPersistedFilter() }

const filterSlice = createSlice({
  name: "filter",
  initialState,
  reducers: {
    setFilter(state, action: PayloadAction<FilterState>) {
      state.filter = action.payload
    },
    patchFilter(state, action: PayloadAction<Partial<FilterState>>) {
      Object.assign(state.filter, action.payload)
    },
    clearFilterKey(
      state,
      action: PayloadAction<Exclude<keyof FilterState, "range_preset">>,
    ) {
      state.filter[action.payload] = ""
    },
    resetFilter(state) {
      state.filter = EMPTY_FILTER
    },
  },
})

export const { setFilter, patchFilter, clearFilterKey, resetFilter } =
  filterSlice.actions
export default filterSlice.reducer
