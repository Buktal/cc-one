// Sessions browser state + actions, extracted from SessionsView so the
// component shrinks to pure rendering. Owns: the tab / search / sidebar-group
// selection, the session list + groups + devices queries, the transcript query
// for the open detail sheet, optimistic favorite toggling, the two-track group
// CRUD (local = immediate, synced = async git push → optimistic + loading), and
// the derived sidebar buckets / visible-session list.
//
// vitest runs in a node-only environment (no DOM — see vitest.config.ts), so
// renderHook is out of scope; the companion test guards that this module
// imports cleanly in node (it pulls the tauri-specta API + RTK Query hooks).

import dayjs from "dayjs"
import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useAppInfoQuery,
  useCreateLocalGroupMutation,
  useCreateSyncedGroupMutation,
  useDeleteLocalGroupMutation,
  useDeleteSyncedGroupMutation,
  useDevicesQuery,
  useListGroupsQuery,
  useListSessionsQuery,
  useRenameLocalGroupMutation,
  useRenameSyncedGroupMutation,
  useReorderLocalGroupsMutation,
  useReorderSyncedGroupsMutation,
  useSessionTranscriptQuery,
  useSetSessionCustomTitleMutation,
  useSetSessionFavoritedMutation,
  useSetSessionLocalGroupMutation,
  useSetSessionSyncedGroupMutation,
} from "@/app/store/api"
import { useAppDispatch } from "@/app/store/hooks"
import { setView } from "@/app/store/slices/viewSlice"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { effectiveDays, type Preset, presetDays } from "@/lib/date-range"
import { usePersistedState } from "@/lib/persistence"
import type { SessionGroup, SessionRow } from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  applyGroupOrder,
  canCreateSyncedGroup,
  effectiveFavorite,
  favKey,
  filterSessionsByQuery,
  type GroupTrack,
  groupSessionsByGroup,
  nextFavValue,
  type SessionTab,
  selectSessions,
  sessionTabFilter,
  sortSessions,
  withFavOverride,
  withoutFavOverride,
} from "./derive"

/** Persisted-tab key — the chosen tab (local / favorites) survives restarts. */
const TAB_KEY = "cc-one:sessions-tab"

export function useSessionsBrowser() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const [tab, setTab] = usePersistedState<SessionTab>(TAB_KEY, "local")
  const [search, setSearch] = useState("")
  const [source, setSource] = useState("")
  const [model, setModel] = useState("")
  // Time-range filter (mirrors the logs ControlBar): a dynamic preset (today /
  // 7d / 30d / all) is the source of truth — its day bounds are computed on
  // selection. "custom" keeps the user-picked days verbatim.
  const [rangePreset, setRangePresetState] = useState<Preset>("all")
  const [fromDay, setFromDay] = useState("")
  const [toDay, setToDay] = useState("")
  // Device filter (Favorites tab only — narrows "all devices" to one).
  const [deviceScope, setDeviceScope] = useState("")
  // Tick counter for the cross-midnight rollover interval (see below).
  const [, setTick] = useState(0)
  const [selectedGroupId, setSelectedGroupId] = useState<string>(ALL_GROUPS)
  const [favOverrides, setFavOverrides] = useState<Record<string, boolean>>({})
  const [pendingGroup, setPendingGroup] = useState<string | null>(null)
  const [busyGroupId, setBusyGroupId] = useState<string | null>(null)
  // Detail-sheet target stored as a composite key (device_id, id), not a row
  // snapshot. A snapshot goes stale the moment a favorite toggle's refetch
  // clears the optimistic override map — effectiveFavorite would then fall
  // back to the snapshot's old `favorited`, making the sheet's star flicker
  // back to its pre-toggle state. The derived `preview` (below) resolves this
  // key against the live sessions array every render, so it always carries the
  // freshest row.
  const [previewKey, setPreviewKey] = useState<{
    id: string
    device_id: string
  } | null>(null)
  // Last row seen for the open preview — fallback when the session leaves the
  // current slice (tab switch / filter change) so the sheet stays open instead
  // of snapping shut. Refreshed whenever the live lookup hits.
  const lastKnownRef = useRef<SessionRow | null>(null)
  const [editTitle, setEditTitle] = useState(false)
  const [titleDraft, setTitleDraft] = useState("")
  const [createGroupOpen, setCreateGroupOpen] = useState(false)

  // The sidebar selection is track-scoped (local vs synced group ids are
  // disjoint spaces), so a tab switch must drop a stale selection.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — reset the group selection on tab switch; the body needs no tab value
  useEffect(() => {
    setSelectedGroupId(ALL_GROUPS)
  }, [tab])

  const { data: appInfo } = useAppInfoQuery()
  const selfDeviceId = appInfo?.device_id ?? ""
  const synced = appInfo?.mode === "synced"
  const effectiveTrack: GroupTrack = tab === "local" ? "local" : "synced"

  // Time-range filter: picking a dynamic preset (today/7d/30d) computes the
  // concrete day bounds on the spot; "all" clears them. Manual date edits flip
  // to "custom". The days (not the preset label) are the filter source of truth,
  // matching the usage view's pattern.
  function setRangePreset(p: Preset): void {
    setRangePresetState(p)
    const days = p === "all" ? { from_day: "", to_day: "" } : presetDays(p)
    setFromDay(days.from_day)
    setToDay(days.to_day)
  }
  function patchFromDay(d: string): void {
    setRangePresetState("custom")
    setFromDay(d)
  }
  function patchToDay(d: string): void {
    setRangePresetState("custom")
    setToDay(d)
  }
  // Local-day range → inclusive ISO8601 timestamp bounds on last_active_at.
  // effectiveDays recomputes a dynamic preset (today/7d/30d) on every render,
  // so a preset picked yesterday rolls to the current day — the stored days
  // are only the frozen selection-time snapshot.
  const effective = effectiveDays({
    range_preset: rangePreset,
    from_day: fromDay,
    to_day: toDay,
  })
  const fromTs = effective.from_day
    ? dayjs(effective.from_day).startOf("day").toISOString()
    : null
  const toTs = effective.to_day
    ? dayjs(effective.to_day).endOf("day").toISOString()
    : null
  // Cross-midnight rollover: effectiveDays runs on render, but with no user
  // input or query refetch there is no render — tick once a minute so the
  // bounds flip to the new day without a reload.
  useEffect(() => {
    const id = setInterval(() => setTick((n) => n + 1), 60_000)
    return () => clearInterval(id)
  }, [])

  // Sessions for the active tab, narrowed by the toolbar filters (time / source
  // / device). All narrowing is backend-side (single source of truth) — the
  // substring search box below is a separate client-side concern. Skipped until
  // selfDeviceId resolves so the local tab never queries with an empty
  // device_scope.
  const sessionsQuery = useListSessionsQuery(
    sessionTabFilter(tab, selfDeviceId, {
      source: source || null,
      fromTs,
      toTs,
      deviceScope: deviceScope || null,
      model: model || null,
    }),
    {
      skip: !selfDeviceId,
    },
  )
  const sessions = sessionsQuery.data ?? []
  const groupsQuery = useListGroupsQuery()
  const groups = groupsQuery.data ?? []
  const { data: devices = [] } = useDevicesQuery()
  const transcriptQuery = useSessionTranscriptQuery(
    previewKey
      ? { id: previewKey.id, deviceId: previewKey.device_id }
      : { id: "", deviceId: "" },
    { skip: !previewKey },
  )

  // Drop optimistic overrides the moment fresh list data lands — the write's
  // invalidation triggered a refetch, so the real favorited value is now in.
  const sessionsData = sessionsQuery.data
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — clear overrides when fresh query data arrives; the body needs no sessionsData value
  useEffect(() => {
    setFavOverrides({})
  }, [sessionsData])

  // Same pattern for the group-drag override: cleared when the reorder's
  // invalidation delivers the real order.
  const [groupOrderOverride, setGroupOrderOverride] = useState<string[] | null>(
    null,
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — clear overrides when fresh query data arrives; the body needs no groupsData value
  useEffect(() => {
    setGroupOrderOverride(null)
  }, [groupsQuery.data])

  const [favoritedMut] = useSetSessionFavoritedMutation()
  const [customTitleMut] = useSetSessionCustomTitleMutation()
  const [setLocalGroupMut] = useSetSessionLocalGroupMutation()
  const [setSyncedGroupMut] = useSetSessionSyncedGroupMutation()
  const [createLocalMut] = useCreateLocalGroupMutation()
  const [renameLocalMut] = useRenameLocalGroupMutation()
  const [deleteLocalMut] = useDeleteLocalGroupMutation()
  const [createSyncedMut] = useCreateSyncedGroupMutation()
  const [renameSyncedMut] = useRenameSyncedGroupMutation()
  const [deleteSyncedMut] = useDeleteSyncedGroupMutation()
  const [reorderLocalMut] = useReorderLocalGroupsMutation()
  const [reorderSyncedMut] = useReorderSyncedGroupsMutation()
  const runWithToast = useMutateWithToast()

  // ---- derived read model (pure functions from ./derive) ----
  // Natural order comes sorted from the backend; the override re-sorts it
  // optimistically while a drag's write is in flight.
  const trackGroups = useMemo(
    () =>
      applyGroupOrder(
        groups.filter((g) => g.kind === effectiveTrack),
        groupOrderOverride,
      ),
    [groups, effectiveTrack, groupOrderOverride],
  )
  const sorted = useMemo(() => sortSessions(sessions), [sessions])
  const filtered = useMemo(
    () => filterSessionsByQuery(sorted, search),
    [sorted, search],
  )
  const grouped = useMemo(
    () => groupSessionsByGroup(filtered, groups, effectiveTrack),
    [filtered, groups, effectiveTrack],
  )
  const visibleSessions = useMemo(
    () => selectSessions(filtered, grouped, selectedGroupId),
    [filtered, grouped, selectedGroupId],
  )

  // sessions lookup by composite key — O(1) resolve for the derived preview.
  // Reuses the favKey shape ("device_id/id") so favorite + preview agree on
  // identity (a session is uniquely (device_id, id)).
  const sessionsByKey = useMemo(() => {
    const m = new Map<string, SessionRow>()
    for (const s of sessions) m.set(favKey(s), s)
    return m
  }, [sessions])

  // Derived preview: resolve the open key against the live sessions array
  // every render. After a favorite toggle's refetch this picks up the fresh
  // row immediately, so effectiveFavorite(preview) reflects the new value
  // instead of flickering back to a stale snapshot. Falls back to the
  // last-known row when the session has left the current slice (tab switch /
  // filter) so the detail sheet stays open.
  const livePreview = useMemo<SessionRow | null>(() => {
    if (!previewKey) return null
    return sessionsByKey.get(favKey(previewKey)) ?? null
  }, [previewKey, sessionsByKey])
  // Refresh the fallback only on a live hit; when the row leaves the slice the
  // fallback keeps the previous row so the sheet does not snap shut.
  useEffect(() => {
    if (livePreview) lastKnownRef.current = livePreview
  }, [livePreview])
  const preview = previewKey ? (livePreview ?? lastKnownRef.current) : null

  // setPreview keeps the caller contract (SessionRow | null) but stores only
  // the composite key — so the transcript query and title/favorite lookups
  // keep working even after a tab switch or filter change removes the row
  // from the visible list.
  function setPreview(s: SessionRow | null): void {
    if (s) {
      lastKnownRef.current = s
      setPreviewKey({ id: s.id, device_id: s.device_id })
    } else {
      lastKnownRef.current = null
      setPreviewKey(null)
    }
  }

  // id → display label for the favorites tab's source-device column. Self is
  // "This device"; a peer is its display name (or "Unnamed").
  const deviceLabel = useMemo(() => {
    const m = new Map<string, string>()
    for (const d of devices) {
      m.set(
        d.device_id,
        d.is_self
          ? t("devices.thisDevice")
          : d.display_name || t("common.unnamed"),
      )
    }
    return m
  }, [devices, t])
  // Device-picker options for the Favorites-tab device filter. Same label logic
  // as deviceLabel; empty when ≤1 device so a single-machine setup renders no
  // device filter (mirrors the usage view's useDeviceOptions).
  const deviceOptions = useMemo(
    () =>
      devices.length <= 1
        ? []
        : devices.map((d) => ({
            id: d.device_id,
            label: d.is_self
              ? t("devices.thisDevice")
              : d.display_name || t("common.unnamed"),
          })),
    [devices, t],
  )
  // The device column only matters in the favorites tab, and only when there
  // is more than one device (otherwise every row is "This device" — noise).
  const showDeviceColumn = tab === "favorites" && devices.length > 1

  // ---- session row actions ----
  // Favorite toggle is an optimistic state machine (stamp → mutate → rollback
  // on failure); the decisions live in ./derive (effectiveFavorite /
  // nextFavValue / withFavOverride / withoutFavOverride) so they are unit-
  // tested. This hook only wires them to React state + the mutation.
  async function toggleFavorite(s: SessionRow): Promise<void> {
    const next = nextFavValue(s, favOverrides)
    setFavOverrides((p) => withFavOverride(p, s, next))
    const ok = await runWithToast(
      favoritedMut,
      { id: s.id, deviceId: s.device_id, favorited: next },
      {
        success: {
          key: next ? "sessions.toast.favorited" : "sessions.toast.unfavorited",
        },
        failed: { key: "sessions.toast.failed" },
      },
    )
    if (!ok) {
      // Rollback the optimistic flip.
      setFavOverrides((p) => withoutFavOverride(p, s))
    }
  }

  async function setSessionGroup(
    s: SessionRow,
    groupId: string | null,
  ): Promise<void> {
    const mut =
      effectiveTrack === "local" ? setLocalGroupMut : setSyncedGroupMut
    await runWithToast(
      mut,
      { id: s.id, deviceId: s.device_id, groupId },
      {
        success: { key: "sessions.toast.groupAssigned" },
        failed: { key: "sessions.toast.failed" },
      },
    )
  }

  // ---- detail sheet: title rename ----
  function startEditTitle(): void {
    if (!preview) return
    setEditTitle(true)
    setTitleDraft(preview.title)
  }
  function cancelEditTitle(): void {
    setEditTitle(false)
  }
  async function commitEditTitle(): Promise<void> {
    if (!preview) return
    const name = titleDraft.trim()
    // Empty draft = revert to the original title (clears the custom override).
    if (!name || name === preview.title) {
      setEditTitle(false)
      return
    }
    const ok = await runWithToast(
      customTitleMut,
      { id: preview.id, deviceId: preview.device_id, title: name },
      {
        success: { key: "sessions.toast.renamed" },
        failed: { key: "sessions.toast.failed" },
      },
    )
    if (ok) setEditTitle(false)
  }

  // ---- group CRUD ----
  // Local groups are immediate (SQLite); synced groups round-trip through git
  // push, so the UI shows an optimistic pending row + spinner until the write
  // resolves (ADR 0002).
  //
  // Synced (Favorites-tab) groups need a bound Git repo — without one the
  // create would silently fail or hang. openCreateGroup is the UX guard (toast
  // + a one-hop to Settings, never opens the dialog); createGroup re-checks
  // defensively in case a caller bypasses the opener.
  function notifyGitRequired(): void {
    toast.warning(t("sessions.group.gitRequiredTitle"), {
      description: t("sessions.group.gitRequiredDesc"),
      action: {
        label: t("sessions.group.gitRequiredAction"),
        onClick: () => dispatch(setView("settings")),
      },
    })
  }
  function openCreateGroup(): void {
    if (!canCreateSyncedGroup(effectiveTrack, synced)) {
      notifyGitRequired()
      return
    }
    setCreateGroupOpen(true)
  }
  async function createGroup(name: string): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) return false
    if (!canCreateSyncedGroup(effectiveTrack, synced)) {
      notifyGitRequired()
      return false
    }
    // Branch per track so the toast helper can infer a single return type
    // (createLocal returns LocalGroup, createSynced returns SyncedGroup — a
    // union trigger would not unify).
    setPendingGroup(trimmed)
    const ok =
      effectiveTrack === "local"
        ? await runWithToast(createLocalMut, trimmed, {
            success: { key: "sessions.toast.groupCreated" },
            failed: { key: "sessions.toast.failed" },
          })
        : await runWithToast(createSyncedMut, trimmed, {
            success: { key: "sessions.toast.groupCreated" },
            failed: { key: "sessions.toast.failed" },
          })
    setPendingGroup(null)
    if (ok) setCreateGroupOpen(false)
    return ok
  }

  async function renameGroup(g: SessionGroup, name: string): Promise<void> {
    const trimmed = name.trim()
    if (!trimmed || trimmed === g.name) return
    setBusyGroupId(g.id)
    try {
      const mut = g.kind === "local" ? renameLocalMut : renameSyncedMut
      await runWithToast(
        mut,
        { id: g.id, name: trimmed },
        {
          success: { key: "sessions.toast.groupRenamed" },
          failed: { key: "sessions.toast.failed" },
        },
      )
    } finally {
      setBusyGroupId(null)
    }
  }

  async function deleteGroup(g: SessionGroup): Promise<void> {
    setBusyGroupId(g.id)
    try {
      const mut = g.kind === "local" ? deleteLocalMut : deleteSyncedMut
      const ok = await runWithToast(mut, g.id, {
        success: { key: "sessions.toast.groupDeleted" },
        failed: { key: "sessions.toast.failed" },
      })
      if (ok && selectedGroupId === g.id) setSelectedGroupId(ALL_GROUPS)
    } finally {
      setBusyGroupId(null)
    }
  }

  // Group drag-reorder: optimistic stamp → mutate → snap back on failure. A
  // drag must not visibly snap while the write is in flight (synced reorders
  // round-trip through git), and the outcome is already visible to the user —
  // no success toast.
  async function reorderGroups(orderedIds: string[]): Promise<void> {
    setGroupOrderOverride(orderedIds)
    const mut = effectiveTrack === "local" ? reorderLocalMut : reorderSyncedMut
    const ok = await runWithToast(mut, orderedIds, {
      failed: { key: "sessions.toast.failed" },
    })
    if (!ok) setGroupOrderOverride(null)
  }

  return {
    // tab / search / source / model / selection
    tab,
    setTab,
    search,
    setSearch,
    source,
    setSource,
    model,
    setModel,
    selectedGroupId,
    setSelectedGroupId,
    effectiveTrack,
    // toolbar filters (time range · device)
    rangePreset,
    fromDay,
    toDay,
    setRangePreset,
    setFromDay: patchFromDay,
    setToDay: patchToDay,
    deviceScope,
    setDeviceScope,
    deviceOptions,
    // data
    sessions,
    isLoading: sessionsQuery.isLoading,
    error: sessionsQuery.error,
    trackGroups,
    grouped,
    visibleSessions,
    totalCount: filtered.length,
    // device labels (favorites tab)
    deviceLabel,
    showDeviceColumn,
    // session row actions
    effectiveFavorite: (s: SessionRow) => effectiveFavorite(s, favOverrides),
    toggleFavorite,
    setSessionGroup,
    // detail sheet
    preview,
    setPreview,
    transcript: transcriptQuery.data ?? [],
    transcriptLoading: transcriptQuery.isLoading,
    transcriptError: transcriptQuery.error,
    refetchTranscript: transcriptQuery.refetch,
    editTitle,
    titleDraft,
    setTitleDraft,
    startEditTitle,
    cancelEditTitle,
    commitEditTitle,
    // group CRUD
    createGroupOpen,
    setCreateGroupOpen,
    openCreateGroup,
    createGroup,
    renameGroup,
    deleteGroup,
    reorderGroups,
    pendingGroup,
    busyGroupId,
  }
}

export type UseSessionsBrowser = ReturnType<typeof useSessionsBrowser>
