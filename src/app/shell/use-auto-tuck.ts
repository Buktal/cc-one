// Auto-tuck: when the FULL window is invisible (minimized, or hidden to the
// tray via close_behavior = minimize) for a configured delay, the same window
// morphs into the tucked mini bar — the user comes back to today's token
// total on screen instead of a buried taskbar/tray entry. Delay lives in
// Settings (lightweight_auto_tuck_secs, 0 = off).
//
// Deliberately EVENT-DRIVEN, not geometry-driven: the earlier "drag to edge" /
// "mouse off card" auto-tucks were the flicker / DPI / SetWindowPos⇄onMoved
// loop bug source (see use-lightweight-tuck.ts). Minimize / hide both force a
// WM_KILLFOCUS, so onFocusChanged is a reliable one-shot signal — no polling,
// and no continuous geometry detection to re-enter that loop. The tuck itself
// dispatches the same store transition the title bar's →小 button does, so the
// existing dock path (useLightweightTuck) handles the SetWindowPos.
//
// Cancellation: any focus (taskbar restore, tray show) clears the timer; the
// fire handler re-checks visibility so a tray show that never delivers focus
// still can't morph a window the user is already looking at.
//
// getCurrentWindow() is fetched lazily inside the effect (not at module top),
// so importing this hook stays safe in a non-Tauri (vitest) environment — same
// pattern as use-tuck-drag / use-lightweight-tuck.

import { getCurrentWindow } from "@tauri-apps/api/window"
import { useEffect, useRef } from "react"

import { usePreferencesQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  setLightweightPhase,
  setMode,
  type WindowMode,
} from "@/app/store/slices/viewSlice"

export interface AutoTuckDecision {
  mode: WindowMode
  /** Delay in seconds before tucking; 0 = off. */
  delaySecs: number
  minimized: boolean
  visible: boolean
}

/** Pure decision: should an auto-tuck timer start for the current window
 *  state? Only the full window tucks (lightweight is already docked), only
 *  with a configured delay, and only while actually invisible. */
export function shouldAutoTuck({
  mode,
  delaySecs,
  minimized,
  visible,
}: AutoTuckDecision): boolean {
  return mode === "full" && delaySecs > 0 && (minimized || !visible)
}

export function useAutoTuck() {
  const dispatch = useAppDispatch()
  const mode = useAppSelector((s) => s.view.mode)
  const { data: prefs } = usePreferencesQuery()
  // Mirror the live values into refs so the event handlers read the latest
  // mode / delay without stale-closure dependencies (same idiom as
  // use-lightweight-tuck's phaseRef).
  const modeRef = useRef(mode)
  modeRef.current = mode
  const delayRef = useRef(prefs?.lightweight_auto_tuck_secs ?? 0)
  delayRef.current = prefs?.lightweight_auto_tuck_secs ?? 0
  const timer = useRef<number | null>(null)

  useEffect(() => {
    if (mode !== "full") return
    const appWindow = getCurrentWindow()
    let unlisten: (() => void) | null = null

    void appWindow
      .onFocusChanged(({ payload: focused }) => {
        void (async () => {
          if (focused) {
            // Restore / tray-show while counting: abort the pending tuck.
            if (timer.current != null) {
              window.clearTimeout(timer.current)
              timer.current = null
            }
            return
          }
          // Minimize and hide-to-tray both land here (WM_KILLFOCUS). A plain
          // focus loss to another app stays visible — not a tuck candidate.
          if (timer.current != null) return
          if (
            !shouldAutoTuck({
              mode: modeRef.current,
              delaySecs: delayRef.current,
              minimized: await appWindow.isMinimized(),
              visible: await appWindow.isVisible(),
            })
          )
            return
          timer.current = window.setTimeout(() => {
            timer.current = null
            void (async () => {
              // Re-check before morphing: the user may have restored the window
              // while the timer ran (a tray show that didn't deliver focus).
              if (modeRef.current !== "full") return
              if (await appWindow.isMinimized()) await appWindow.unminimize()
              if (!(await appWindow.isVisible())) await appWindow.show()
              dispatch(setMode("lightweight"))
              dispatch(setLightweightPhase("tucked"))
            })()
          }, delayRef.current * 1000)
        })()
      })
      .then((u) => {
        unlisten = u
      })

    return () => {
      if (timer.current != null) window.clearTimeout(timer.current)
      timer.current = null
      unlisten?.()
    }
  }, [mode, dispatch])
}
