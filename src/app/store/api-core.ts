// RTK Query data-layer core over the typed Tauri command contract. Owns the
// base api instance — with NO endpoints: the feature modules inject theirs via
// `vaultApi.injectEndpoints` (src/features/*/api.ts), and src/app/store/api.ts
// is the public seam re-exporting every hook. This module imports no feature
// code, so the data layer never reaches up into features.
//
// Every command returns a `{ status: "ok" | "error" }` envelope (tauri-specta).
// `run` unwraps it into a discriminated `RunResult` — `{ data }` on ok or
// `{ error: AppError }` on error — which queryFns return verbatim. RTK Query
// stores a plain-object error returned from `queryFn` as-is (it only
// serialises *thrown* Errors into `{ name, message, stack }`), so the typed
// `AppError` reaches the UI intact and `describeError` can map `error.type`
// to an i18n key. The UI never sees SQL or invoke() directly.
//
// `fakeBaseQuery<AppError>` pins the endpoint error type so `result.error` is
// always `AppError` (not `unknown`) at call sites.

import { createApi, fakeBaseQuery } from "@reduxjs/toolkit/query/react"

import type {
  AppError,
  CloseBehavior,
  UsageStats,
} from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

type Envelope<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: AppError }

/** Outcome of `run`: the unwrapped payload, or the backend's typed error. */
export type RunResult<T> = { data: T } | { error: AppError }

export async function run<T>(p: Promise<Envelope<T>>): Promise<RunResult<T>> {
  let r: Envelope<T>
  try {
    r = await p
  } catch (e) {
    // tauri-specta's `typedError` re-throws JS-level Errors (e.g. an invoke
    // failure or a Rust panic) instead of wrapping them in the envelope.
    // Normalise those into `Internal` so `run` never throws and the endpoint
    // error type stays honestly `AppError` — callers always get a typed result.
    return {
      error: {
        type: "Internal",
        data: e instanceof Error ? e.message : String(e),
      },
    }
  }
  if (r.status === "ok") return { data: r.data }
  return { error: r.error }
}

/** Zero-value UsageStats — shared UI fallback for loading/empty. */
export const ZERO_STATS: UsageStats = {
  request_count: 0,
  total_tokens: 0,
  input_tokens: 0,
  output_tokens: 0,
  cache_creation_tokens: 0,
  cache_read_tokens: 0,
  cache_hit_rate: 0,
  total_cost_usd: 0,
  turn_count: 0,
  avg_turn_duration_ms: 0,
}

/**
 * The single RTK Query api over the Tauri commands. `tagTypes` is the shared
 * cache-tag registry (see ./tags.ts) that every injected endpoint tags
 * against; the endpoints themselves are injected per feature.
 */
export const vaultApi = createApi({
  reducerPath: "vaultApi",
  baseQuery: fakeBaseQuery<AppError>(),
  tagTypes: [
    "Usage",
    "Logs",
    "Models",
    "Devices",
    "Pricing",
    "Library",
    "Sessions",
    "Providers",
    "App",
    "Store",
  ],
  endpoints: () => ({}),
})

export type VaultApi = typeof vaultApi

/**
 * Resolve the one-time close dialog. Not an RTK Query endpoint —
 * it is a one-shot action (hide window / exit app). `remember` pins `choice`.
 * The sole caller fire-and-forgets this; on the rare error path the structured
 * `AppError` is thrown so a future caller could `describeError` it.
 */
export async function confirmClose(
  choice: CloseBehavior,
  remember: boolean,
): Promise<void> {
  const r = await run(commands.confirmClose(choice, remember))
  if ("error" in r) throw r.error
}
