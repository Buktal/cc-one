// Shared usage-filter state — the single source of truth for the COMMON query
// dimensions (time range / model / source / device scope) shared across the
// dashboard, request-log and sessions views. Empty string = "no constraint";
// toFilter() converts to the nullable UsageFilter the API expects.
//
// Dynamic time presets (today/7d/30d) store NO concrete dates — from_day /
// to_day stay empty and effectiveDays re-derives them from the current date
// inside each endpoint's queryFn. Dates are thus never frozen into state or
// the cache key, so a dynamic preset rolls to the new day via the collect-
// interval refresh chain with no timer of its own. Only custom / all keep
// literal days.
//
// The filter is NOT persisted: every app start resets to DEFAULT_FILTER ("today").

import { createSlice, type PayloadAction } from "@reduxjs/toolkit"

import { type DayRange, dayRangeToTs, effectiveDays } from "@/lib/date-range"
import type { UsageFilter } from "@/types/generated/bindings"

export interface FilterState extends DayRange {
  model: string
  source: string
  device_scope: string
}

/** Default filter — "today", unconstrained otherwise. Not persisted: each app
 *  start begins here. */
export const DEFAULT_FILTER: FilterState = {
  range_preset: "today",
  from_day: "",
  to_day: "",
  model: "",
  source: "",
  device_scope: "",
}

/** "All time" probe — unconstrained on every dimension, used to decide whether
 *  the source dimension should render at all (regardless of the user's current
 *  "today" window). DEFAULT_FILTER is "today", so it is NOT the null baseline —
 *  use this when you mean "no constraints". */
export const ALL_TIME_FILTER: FilterState = {
  ...DEFAULT_FILTER,
  range_preset: "all",
}

/** Convert internal FilterState (empty = no constraint) → API UsageFilter (null).
 *  Date bounds are derived via effectiveDays: a dynamic preset (today/7d/30d)
 *  re-rolls to the current day on every call, so the caller (an endpoint
 *  queryFn) always gets fresh bounds with nothing time-shaped frozen into the
 *  state or cache key. */
export function toFilter(s: FilterState): UsageFilter {
  const { from_day, to_day } = effectiveDays(s)
  const { from_ts, to_ts } = dayRangeToTs(from_day, to_day)
  return {
    from_ts,
    to_ts,
    model: s.model || null,
    source: s.source || null,
    device_scope: s.device_scope || null,
  }
}

interface FilterSliceState {
  filter: FilterState
}

const initialState: FilterSliceState = { filter: DEFAULT_FILTER }

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
      state.filter = DEFAULT_FILTER
    },
  },
})

export const { setFilter, patchFilter, clearFilterKey, resetFilter } =
  filterSlice.actions
export default filterSlice.reducer
