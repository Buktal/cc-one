// Data freshness hook. Tracks the last time the
// Local Store was written (collect / sync) so the cockpit can show "采集于 3 分钟前".
//
// Source of truth for "data was written" is the Tauri `usage_changed` event
// (providers.tsx already invalidates Usage cache on it). Here we *also* persist
// a timestamp to localStorage keyed by device_id, so the hint survives reloads
// and degrades gracefully when no log rows exist yet.
//
// State is held in a module-level Map + useSyncExternalStore. getSnapshot reads
// only from the in-memory map (never re-parses localStorage) so the reference
// stays stable across renders when nothing changed.

import { useEffect, useSyncExternalStore } from "react"

import { listenAppEvent, USAGE_CHANGED } from "@/app/app-events"
import { useAppInfoQuery } from "@/app/store/api"
import { debouncedLocalStorageWrite } from "@/lib/persistence"

export interface FreshnessState {
  /** epoch ms of last successful collect (or null if never). */
  lastCollectAt: number | null
  /** epoch ms of last successful sync (or null if never / Standalone). */
  lastSyncAt: number | null
}

const NULL_STATE: FreshnessState = { lastCollectAt: null, lastSyncAt: null }

const stores = new Map<string, FreshnessState>()
const listeners = new Set<() => void>()

function storageKey(deviceId: string) {
  return `cc-one:freshness:${deviceId}`
}

function readStorage(deviceId: string): FreshnessState {
  try {
    const raw = localStorage.getItem(storageKey(deviceId))
    if (!raw) return { ...NULL_STATE }
    const parsed = JSON.parse(raw) as Partial<FreshnessState>
    return {
      lastCollectAt: parsed.lastCollectAt ?? null,
      lastSyncAt: parsed.lastSyncAt ?? null,
    }
  } catch {
    return { ...NULL_STATE }
  }
}

function ensure(deviceId: string): FreshnessState {
  let s = stores.get(deviceId)
  if (!s) {
    s = readStorage(deviceId)
    stores.set(deviceId, s)
  }
  return s
}

// NOTE: this module is one of the two documented exceptions to
// usePersistedState (see src/lib/persistence.ts). Freshness state is shared
// LIVE across multiple hooks/components, which needs useSyncExternalStore over
// a module Map; it reuses only the debounced writer primitive to mirror the
// in-memory map to disk.
function write(deviceId: string, next: FreshnessState) {
  stores.set(deviceId, next)
  // Debounced disk mirror of the in-memory map. The map is the session source
  // of truth; this just survives reloads. Writes are infrequent (collect/sync
  // edges) so the 300ms window is harmless, and the persistence module flushes
  // on beforeunload.
  debouncedLocalStorageWrite(storageKey(deviceId), next)
  for (const l of listeners) l()
}

function mark(deviceId: string, field: "lastCollectAt" | "lastSyncAt") {
  const cur = ensure(deviceId)
  write(deviceId, { ...cur, [field]: Date.now() })
}

function subscribe(cb: () => void) {
  listeners.add(cb)
  return () => {
    listeners.delete(cb)
  }
}

/**
 * Subscribe to the device's data-freshness state. Mounts a `usage_changed`
 * listener that bumps `lastCollectAt` automatically; callers should also call
 * `markCollected()`/`markSynced()` on their own mutation success for an
 * immediate hint even when the ingest produced no new rows.
 */
export function useFreshness() {
  const { data: info } = useAppInfoQuery(undefined, { pollingInterval: 0 })
  const deviceId = info?.device_id ?? null

  useEffect(() => {
    if (!deviceId) return
    ensure(deviceId)
    let unlisten: (() => void) | null = null
    listenAppEvent(USAGE_CHANGED, () => mark(deviceId, "lastCollectAt")).then(
      (u) => {
        unlisten = u
      },
    )
    return () => {
      unlisten?.()
    }
  }, [deviceId])

  const state = useSyncExternalStore(
    subscribe,
    () => (deviceId ? ensure(deviceId) : NULL_STATE),
    () => (deviceId ? ensure(deviceId) : NULL_STATE),
  )

  return {
    state,
    markCollected: () => {
      if (deviceId) mark(deviceId, "lastCollectAt")
    },
    markSynced: () => {
      if (deviceId) mark(deviceId, "lastSyncAt")
    },
  }
}
