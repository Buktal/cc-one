// Shared usage-filter state — the single source of truth for the COMMON query
// dimensions (time range / model / source / device scope / project) shared
// across the dashboard, request-log and sessions views. Empty string = "no
// constraint"; toFilter() converts to the nullable UsageFilter the API expects.
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

import {
  type DayRange,
  dayRangeToTs,
  effectiveDays,
  type Preset,
} from "@/lib/date-range"
import type { UsageFilter } from "@/types/generated/bindings"

export interface FilterState extends DayRange {
  model: string
  source: string
  device_scope: string
  /** Project identity ("" = no constraint). The unknown-project sentinel may
   *  land here as a value — it arrives as DATA from the distinct-projects
   *  endpoint (`ProjectCandidates.unknown`), never as a frontend literal. */
  project: string
}

/** Every FilterState dimension, in cache-key concatenation order. Cache keys
 *  derived from a FilterState — filterId (features/usage/derive.ts) and
 *  sessionSpecId (features/sessions/derive.ts) — must cover all of these;
 *  features/usage/derive.test.ts turns red when a new dimension is added here
 *  but missed there, so differing dimension values can never silently share a
 *  cache entry. */
export const FILTER_DIMENSIONS = [
  "range_preset",
  "from_day",
  "to_day",
  "model",
  "source",
  "device_scope",
  "project",
] as const satisfies readonly (keyof FilterState)[]

/** Default filter — "today", unconstrained otherwise. Not persisted: each app
 *  start begins here. */
export const DEFAULT_FILTER: FilterState = {
  range_preset: "today",
  from_day: "",
  to_day: "",
  model: "",
  source: "",
  device_scope: "",
  project: "",
}

/** "All time" probe — unconstrained on every dimension, used to decide whether
 *  the source dimension should render at all (regardless of the user's current
 *  "today" window). DEFAULT_FILTER is "today", so it is NOT the null baseline —
 *  use this when you mean "no constraints". */
export const ALL_TIME_FILTER: FilterState = {
  ...DEFAULT_FILTER,
  range_preset: "all",
}

/** 时间范围写的切片补丁（ADR-0008 的写侧，单一归属）：动态预设不存具体日期
 *  ——选预设即清空 from_day/to_day（cache key 一天内稳定，日期在 queryFn 实
 *  时派生）；日期编辑转 custom 并存字面值。DateRangeChip 的两处调用
 *  （usage 与 sessions 工具栏，经 useDateRangeFilter）都经这两个补丁写 slice，
 *  不再各写各的 dispatch 链。 */
export function presetPatch(p: Preset): Partial<FilterState> {
  return { range_preset: p, from_day: "", to_day: "" }
}

export function dayPatch(
  field: "from_day" | "to_day",
  day: string,
): Partial<FilterState> {
  return { range_preset: "custom", [field]: day }
}

/** Convert internal FilterState (empty = no constraint) → API UsageFilter (null).
 *  Date bounds are derived via effectiveDays: a dynamic preset (today/7d/30d)
 *  re-rolls to the current day on every call, so the caller (an endpoint
 *  queryFn) always gets fresh bounds with nothing time-shaped frozen into the
 *  state or cache key. The project value passes through as-is — a known
 *  identity narrows via the EXISTS rule, the unknown sentinel (arriving as
 *  endpoint data) via its NOT-EXISTS / empty-identity semantics; both live
 *  backend-side behind the one `project` field. */
export function toFilter(s: FilterState): UsageFilter {
  const { from_day, to_day } = effectiveDays(s)
  const { from_ts, to_ts } = dayRangeToTs(from_day, to_day)
  return {
    from_ts,
    to_ts,
    model: s.model || null,
    source: s.source || null,
    device_scope: s.device_scope || null,
    project: s.project || null,
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
