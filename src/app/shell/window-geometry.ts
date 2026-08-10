// Persisted window geometry for all three window shapes, in ONE JSON blob so
// each shape's position/state survives both the lightweight unmount (App.tsx
// drops <LightweightCard/> when mode flips to full) and app restarts. One key
// = one source of truth; was previously scattered across ad-hoc keys.
//
// Shapes:
//   - full:     the full dashboard. Stores the OS "restored" rect (x/y/w/h in
//               logical px) plus whether it was maximized. On re-entry we
//               re-maximize or land at the stored rect. null until the user
//               has shaped full-mode once (first entry falls back to center).
//   - expanded: the right-docked glance card. Only Y varies (X is flush-right).
//   - tucked:   the right-docked mini-bar. Only Y varies.
//
// All values are LOGICAL px. Full-mode x/y are relative to the virtual-screen
// origin (outerPosition / scaleFactor); w/h are outer size / scaleFactor. The
// Rust set_window_rect command converts back to physical on restore.
//
// ── Debounced writes + in-memory cache ──
// `onMoved`/`onResized` fire once per pixel during a drag/resize, so a direct
// localStorage write per event is a burst of synchronous setItems. Writes now
// go through the shared `debouncedLocalStorageWrite` primitive (300ms), which
// coalesces a whole drag into one disk write.
//
// Debouncing a write-through cache would lose data: `saveFullRect({x})` then
// `saveFullRect({y})` would both read whatever is on disk, and while the first
// write is still pending the second reads stale data and clobbers x. So the
// canonical record lives in an in-memory `cached` variable, lazily loaded from
// localStorage on first access; every save* mutates that snapshot and
// schedules a debounced mirror to disk. Reads within a session hit the cache
// (always fresh); the disk file is only the cross-session mirror. The hooks
// that drive these writes (useWindowMode, useLightweightTuck) call
// `flushPendingWrites()` on unmount, and the persistence module flushes on
// `beforeunload`, so the trailing debounced value is never lost to a close.

import { debouncedLocalStorageWrite, readPersisted } from "@/lib/persistence"
import { ENTRY_DOCK_Y } from "./lightweight-geometry"

export type FullGeom = {
  maximized: boolean
  x: number
  y: number
  w: number
  h: number
}

export type WindowGeometry = {
  full: FullGeom | null
  expanded: { y: number }
  tucked: { y: number }
}

const KEY = "cc-one:window-geometry"
const DEBOUNCE_MS = 300

function defaults(): WindowGeometry {
  return {
    full: null,
    expanded: { y: ENTRY_DOCK_Y },
    tucked: { y: ENTRY_DOCK_Y },
  }
}

/** In-memory source of truth for the session, lazily seeded from localStorage.
 *  Held outside localStorage so the read-modify-write `save*` helpers always
 *  patch the latest record even while a debounced disk write is in flight. */
let cached: WindowGeometry | undefined

function load(): WindowGeometry {
  if (cached) return cached
  // Disk read funnels through the shared persistence primitive — the save*
  // helpers already mirror writes through it, so reads must not hand-roll a
  // second localStorage path. A null / unparseable record falls back to defaults.
  const p = readPersisted<Partial<WindowGeometry> | null>(KEY, null)
  cached = p
    ? {
        full: p.full ?? null,
        expanded: { y: p.expanded?.y ?? ENTRY_DOCK_Y },
        tucked: { y: p.tucked?.y ?? ENTRY_DOCK_Y },
      }
    : defaults()
  return cached
}

/** Replace the canonical record and debounced-mirror it to disk. */
function persist(next: WindowGeometry): void {
  cached = next
  debouncedLocalStorageWrite(KEY, next, DEBOUNCE_MS)
}

export function readFull(): FullGeom | null {
  return load().full
}

/** Overwrite the full-mode record wholesale (used by the entry snapshot). */
export function saveFull(full: FullGeom): void {
  persist({ ...load(), full })
}

/** Patch the restored-window rect (x/y/w/h) leaving `maximized` as-is. No-op
 *  if no full record exists yet — the entry snapshot establishes it first. */
export function saveFullRect(
  rect: Partial<Pick<FullGeom, "x" | "y" | "w" | "h">>,
): void {
  const g = load()
  if (!g.full) return
  persist({ ...g, full: { ...g.full, ...rect } })
}

/** Flip the maximized flag without touching the restored rect (the rect is
 *  the windowed geometry; maximizing never overwrites it). No-op if there's
 *  no full record yet, or nothing changed. */
export function saveFullMaximized(maximized: boolean): void {
  const g = load()
  if (!g.full || g.full.maximized === maximized) return
  persist({ ...g, full: { ...g.full, maximized } })
}

export function readLwY(phase: "expanded" | "tucked"): number {
  return load()[phase].y
}

export function saveLwY(phase: "expanded" | "tucked", y: number): void {
  persist({ ...load(), [phase]: { y } })
}
