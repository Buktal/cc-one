// Dashboard section scroll (#106): scrollspy over the shell's scroll
// container + the whole-page progress ratio for the sticky tab bar's
// progress line. The scroller is NOT the window — the shell wraps view
// content in a `overflow-auto` div — so it is located by walking the DOM
// from the dashboard root (computed styles are read inside the effect, never
// at module scope, so vitest's node environment can import this file).

import { useEffect, useState } from "react"

/** The scrollspy edge's head allowance: past the sticky bar's bottom, one
 *  section-head line — the highlight flips as the head (not the card body)
 *  crosses the bar. The bar's own height is measured by the caller (it wraps
 *  to a second line at narrower widths) and passed in. */
const HEAD_LINE_PX = 30

/** First scrollable ancestor (overflow-y auto/scroll) of `el`, or null at the
 *  document root. */
export function findScrollParent(el: HTMLElement): HTMLElement | null {
  for (let node = el.parentElement; node; node = node.parentElement) {
    const { overflowY } = window.getComputedStyle(node)
    if (overflowY === "auto" || overflowY === "scroll") return node
  }
  return null
}

/** Pure picker: the id of the LAST section whose measured top sits at or
 *  above `edge` (i.e. has scrolled under the frozen bar), else `fallback`.
 *  `tops` arrive viewport-relative from getBoundingClientRect in the hook —
 *  extracted so the ordering invariant is testable without a DOM. */
export function pickActiveSection<A>(
  tops: readonly { id: A; top: number }[],
  edge: number,
  fallback: A,
): A {
  let active = fallback
  for (const s of tops) {
    if (s.top <= edge) active = s.id
  }
  return active
}

/** Track which section id is current + the page scroll ratio (0–1) for the
 *  ids given. `ids` should be a stable array (module const) so the effect
 *  does not re-attach per render. `stickyBarHeight` is the frozen tab bar's
 *  measured height (ResizeObserver in dashboard-view) — the spy edge tracks
 *  the real bar, not a guessed constant, so a wrapped two-line bar keeps the
 *  highlight and the anchor clears in sync. */
export function useSectionScroll<T extends HTMLElement>(
  rootRef: React.RefObject<T | null>,
  ids: readonly string[],
  stickyBarHeight: number,
) {
  const [activeId, setActiveId] = useState(ids[0] ?? null)
  const [progress, setProgress] = useState(0)

  useEffect(() => {
    const root = rootRef.current
    if (!root) return
    const scroller = findScrollParent(root)
    if (!scroller) return

    const sync = () => {
      const edge =
        scroller.getBoundingClientRect().top + stickyBarHeight + HEAD_LINE_PX
      const tops = ids.map((id) => {
        const el = document.getElementById(id)
        return { id, top: el ? el.getBoundingClientRect().top : Number.NaN }
      })
      // Unresolvable ids (NaN top) never satisfy the edge check, so a section
      // not yet mounted just keeps the previous highlight.
      setActiveId((prev) =>
        pickActiveSection(
          tops.filter((t) => Number.isFinite(t.top)),
          edge,
          prev,
        ),
      )
      const max = scroller.scrollHeight - scroller.clientHeight
      setProgress(max > 0 ? scroller.scrollTop / max : 0)
    }

    sync()
    scroller.addEventListener("scroll", sync, { passive: true })
    return () => scroller.removeEventListener("scroll", sync)
  }, [rootRef, ids, stickyBarHeight])

  return { activeId, progress }
}
