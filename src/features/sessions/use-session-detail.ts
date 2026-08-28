// Detail + neighbor-stepping domain of the sessions browser（架构审查Ⅴ拆分
// 之一）：详情目标的复合键状态（(device_id, id)，不是行快照）、live 行解析
// 与离页回退、transcript 读数、以及 prev/next 邻步（含页缘翻页步进的登记与
// 结算）。步进的判定不变量住在 ./derive（planNeighborStep / settleNeighborStep
// ——已测），本 hook 只把它们接到 React 状态与分页控制器上。
//
// 跨域依赖以显式参数进入：分页控制器的 offset / pageSize / 总数 / 翻页动作
// 由组合根传入——本域不持有分页状态，只在页缘步进时借用。

import { useEffect, useMemo, useRef, useState } from "react"
import { useSessionTranscriptQuery } from "@/app/store/api"
import type { SessionRow } from "@/types/generated/bindings"
import {
  favKey,
  neighborNav,
  type PendingNeighborStep,
  planNeighborStep,
  settleNeighborStep,
} from "./derive"

export interface SessionDetailDomainInput {
  /** 当前页可见行（nestSubagents 后的展示序 = 邻步导航序）。 */
  visibleSessions: readonly SessionRow[]
  /** 分页控制器的行偏移（页缘判定用）。 */
  offset: number
  pageSize: number
  /** 分页总数（页缘「还有下一页」判定用）。 */
  total: number
  /** 分页控制器的按页平移动作（页缘步进翻页用）。 */
  shiftPages: (delta: number) => void
}

export function useSessionDetail({
  visibleSessions,
  offset,
  pageSize,
  total,
  shiftPages,
}: SessionDetailDomainInput) {
  // Detail target stored as a composite key (device_id, id), not a row
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

  const transcriptQuery = useSessionTranscriptQuery(
    previewKey
      ? { id: previewKey.id, deviceId: previewKey.device_id }
      : { id: "", deviceId: "" },
    { skip: !previewKey },
  )

  // sessions lookup by composite key — O(1) resolve for the derived preview.
  // Reuses the favKey shape ("device_id/id") so favorite + preview agree on
  // identity (a session is uniquely (device_id, id)). Only the current page
  // is in memory; a preview whose row left the slice falls back to the
  // last-known row below so the sheet stays open across page turns.
  const sessionsByKey = useMemo(() => {
    const m = new Map<string, SessionRow>()
    for (const s of visibleSessions) m.set(favKey(s), s)
    return m
  }, [visibleSessions])

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

  // ---- detail sheet: prev / next session navigation ----
  // Walks the currently visible page (±1 row); at a page edge it pages into
  // the adjacent page and opens its target row once the new page's data
  // lands. The decisions live in ./derive (planNeighborStep /
  // settleNeighborStep — the stepping invariants are unit-tested there); this
  // hook only registers the pending step, shifts pages, and consumes the
  // settlement when the list data changes.
  const pendingNeighbor = useRef<PendingNeighborStep | null>(null)
  const neighbor = useMemo(
    () =>
      neighborNav(
        visibleSessions,
        previewKey ? favKey(previewKey) : null,
        offset,
        pageSize,
        total,
      ),
    [visibleSessions, previewKey, offset, pageSize, total],
  )

  function openNeighbor(delta: 1 | -1): void {
    const step = planNeighborStep(
      visibleSessions,
      previewKey ? favKey(previewKey) : null,
      delta,
      offset,
      pageSize,
      total,
    )
    if (step.kind === "in-page") {
      setPreview(step.target)
    } else if (step.kind === "page-edge") {
      pendingNeighbor.current = step.pending
      shiftPages(delta)
    }
  }

  // Consume the pending page-edge step when the new page's data lands (the
  // open / drop / wait rules live in settleNeighborStep).
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — setPreview is stable (reads refs only); adding it would re-run the effect every render
  useEffect(() => {
    const p = pendingNeighbor.current
    if (!p) return
    const settled = settleNeighborStep(
      p,
      previewKey ? favKey(previewKey) : null,
      visibleSessions,
    )
    if (settled.kind === "wait") return
    pendingNeighbor.current = null
    if (settled.kind === "open") setPreview(settled.target)
  }, [visibleSessions, previewKey])

  return {
    preview,
    setPreview,
    openNeighbor,
    canPrev: neighbor.canPrev,
    canNext: neighbor.canNext,
    transcript: transcriptQuery.data ?? [],
    transcriptLoading: transcriptQuery.isLoading,
    transcriptError: transcriptQuery.error,
    refetchTranscript: transcriptQuery.refetch,
  }
}

export type SessionDetailDomain = ReturnType<typeof useSessionDetail>
