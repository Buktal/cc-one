// Update Check orchestration. Exposes the side-effect surface:
//   - checkNow:    probe GitHub Releases for a newer version (startup silent
//                  probe on launch + a re-probe every POLL_INTERVAL_MS while the
//                  app stays open; Settings calls this manually). A check()
//                  failure is silent (back to idle; the indicator never shows).
//   - applyUpdate: downloadAndInstall the pending Update. Progress → slice;
//                  success → ready; failure → Manual Fallback (failed).
//   - restartNow:  restart after a ready install (process:allow-restart).
//   - openReleases: open GitHub Releases (footer 📖 button + Manual Fallback).
//
// The pending Update object (returned by check, holds downloadAndInstall) is
// module-level: at most one is in flight at a time and it is shared across hook
// instances — App mounts the startup probe, UpdateCard calls applyUpdate.

import { openUrl } from "@tauri-apps/plugin-opener"
import { relaunch } from "@tauri-apps/plugin-process"
import { check, type Update } from "@tauri-apps/plugin-updater"
import { useCallback, useEffect, useRef } from "react"

import { useAppDispatch } from "@/app/store/hooks"
import {
  setAvailable,
  setChecking,
  setDownloading,
  setFailed,
  setIdle,
  setReady,
  setUpToDate,
} from "@/app/store/slices/updateSlice"
import { toStructuredError } from "@/lib/error"

const RELEASES_URL = "https://github.com/Buktal/cc-one/releases/latest"
// While the app stays open, re-probe this often so a long-lived session still
// catches a new release (startup always probes immediately).
const POLL_INTERVAL_MS = 6 * 60 * 60 * 1000

/** Singleton: the Update found by the last check (holds downloadAndInstall).
 *  A Tauri `Update` carries a non-serializable side-effect
 *  (`downloadAndInstall`) and must be shared app-wide across the App / footer /
 *  Settings hook instances, so it stays a module-level `let`, not persisted
 *  state. */
let pendingUpdate: Update | null = null
/** Singleton: the startup probe (and the re-probe interval armed by it) runs
 *  exactly once app-wide, even though useUpdateCheck is mounted in App + footer
 *  + Settings. */
let startupProbed = false

export function useUpdateCheck() {
  const dispatch = useAppDispatch()
  // Guard against a probe already in flight (startup fire + manual click).
  const inFlight = useRef(false)

  const checkNow = useCallback(async () => {
    if (inFlight.current) return
    inFlight.current = true
    dispatch(setChecking())
    try {
      const update = await check()
      if (update?.available) {
        pendingUpdate = update
        dispatch(
          setAvailable({
            version: update.version,
            currentVersion: update.currentVersion,
            notes: update.body ?? null,
          }),
        )
      } else {
        pendingUpdate = null
        dispatch(setUpToDate())
      }
    } catch {
      // Silent failure: no network, 404 latest.json, endpoint down.
      pendingUpdate = null
      dispatch(setIdle())
    } finally {
      inFlight.current = false
    }
  }, [dispatch])

  const applyUpdate = useCallback(async () => {
    const update = pendingUpdate
    if (!update) return
    let downloaded = 0
    let total = 0
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && "contentLength" in event.data) {
          total = event.data.contentLength ?? 0
          dispatch(setDownloading({ downloadedBytes: 0, totalBytes: total }))
        } else if (event.event === "Progress" && "chunkLength" in event.data) {
          downloaded += event.data.chunkLength
          dispatch(
            setDownloading({ downloadedBytes: downloaded, totalBytes: total }),
          )
        }
      })
      dispatch(setReady())
      await update.close()
    } catch (e) {
      // Manual Fallback: surface the "go to GitHub" card. The structured form
      // keeps the error re-translatable at the render boundary on a language
      // switch; a raw string would freeze the old-language reason.
      dispatch(
        setFailed({
          error: toStructuredError(e) ?? { kind: "raw", message: String(e) },
        }),
      )
    }
  }, [dispatch])

  const restartNow = useCallback(async () => {
    await relaunch()
  }, [])

  const openReleases = useCallback(async () => {
    await openUrl(RELEASES_URL)
  }, [])

  // Startup silent probe — fires once app-wide on every launch (a fresh import
  // per startup resets the module-level guard), then re-probes every
  // POLL_INTERVAL_MS while the app stays open. The interval is armed without a
  // cleanup on purpose: under StrictMode the effect runs mount→unmount→remount,
  // and a cleanup would clear the timer while the guarded remount re-arms
  // nothing — leaving a long-running session with no polling. The root App keeps
  // this hook mounted for the app's lifetime, so the timer lives with the
  // process, not a component. The many mounts share one probe + one interval.
  useEffect(() => {
    if (startupProbed) return
    startupProbed = true
    void checkNow()
    void setInterval(() => void checkNow(), POLL_INTERVAL_MS)
  }, [checkNow])

  return { checkNow, applyUpdate, restartNow, openReleases }
}
