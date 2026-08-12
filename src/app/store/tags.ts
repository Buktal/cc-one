// Cache-tag single source of truth for the Local Store domain.
//
// "Store" is an AGGREGATE tag: every read endpoint whose data the collect /
// sync / forget-device path can rewrite declares it via storeRead(), and every
// whole-Store write invalidates it via INVALIDATE_STORE. The write side
// therefore cannot "forget a domain" — it no longer names domains, it just
// invalidates Store. Subset writes (session CRUD, rebill, pricing, library,
// provider CRUD, device rename, preferences) keep their fine-grained tags and
// stay surgical: invalidating e.g. "Sessions" hits only Sessions-tagged reads,
// never the other Store reads.
//
// Adding a read endpoint that derives from the Local Store? Use storeRead()
// and add its name to STORE_DERIVED_READS below. tags.test.ts forces a
// classification decision on every read endpoint, so it stays red until you do
// — no silent drift. Adding a whole-Store write? Invalidate INVALIDATE_STORE.

import type { TagDescription } from "@reduxjs/toolkit/query"

/** Aggregate tag for everything derived from the Local Store. */
export const STORE_TAG = "Store" as const

/**
 * Provides-helper for Store-derived reads: prepends the aggregate tag so any
 * whole-Store write (collect / sync / forget-device / the `usage_changed`
 * event) refetches this endpoint. Spread the fine-grained tags in any form
 * (string shorthand or `{ type, id }`). Returns a fresh array each call.
 */
export function storeRead<const Tag extends string>(
  ...rest: ReadonlyArray<TagDescription<Tag>>
): TagDescription<Tag | typeof STORE_TAG>[] {
  return [STORE_TAG, ...rest]
}

/**
 * Invalidation list for whole-Store writes. Single source — the collect / sync
 * / forgetDevice mutations and the `usage_changed` event listener all reference
 * this, so the write side cannot "forget" a domain (it no longer names them).
 */
// Mutable array type (not `as const`): RTK Query's `invalidatesTags` mutation
// field accepts readonly, but the `api.util.invalidateTags(...)` action rejects
// it — a mutable type lets both reference INVALIDATE_STORE directly.
export const INVALIDATE_STORE: TagDescription<typeof STORE_TAG>[] = [STORE_TAG]

// ---- Invariant registries (consumed by tags.test.ts) ----
// Every read endpoint MUST appear in EXACTLY ONE of the two read sets below;
// the test fails if a new read endpoint is added without classification.

/**
 * Read endpoints whose data collect / sync / forget-device can change. Each
 * MUST declare providesTags via storeRead(). Providers is included because
 * sync imports peer providers into the Store — without the Store tag, a synced
 * peer provider would not refresh until an unrelated write.
 */
export const STORE_DERIVED_READS: ReadonlySet<string> = new Set([
  // Usage
  "stats",
  "trend",
  "distinctSources",
  "distinctModels",
  // Logs
  "logs",
  "count",
  // Models
  "models",
  // Devices
  "devices",
  // Sessions
  "listSessions",
  "sessionCounts",
  "sessionTranscript",
  "listGroups",
  "listLocalGroups",
  "listSyncedGroups",
  // Providers (sync imports peer providers into the Store)
  "listProviders",
  "getActiveProvider",
  "getCommonConfigSnippet",
])

/**
 * Read endpoints deliberately NOT Store-derived (independent storage: static
 * pricing file, library files, app config, or tag-less probes). MUST NOT use
 * storeRead().
 */
export const NON_STORE_READS: ReadonlySet<string> = new Set([
  "appInfo",
  "pricing",
  "scanLibrary",
  "libraryDeviceSummary",
  "libraryText",
  "preferences",
])

/** Whole-Store write endpoints. Each MUST invalidate via INVALIDATE_STORE. */
export const WHOLE_STORE_WRITES: ReadonlySet<string> = new Set([
  "collect",
  "sync",
  "forgetDevice",
])
