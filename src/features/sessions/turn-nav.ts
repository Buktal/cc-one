// Turn-nav routing — the "which user turn is currently active" decision for the
// session-detail transcript + its turn-nav panel, as a pure reducer. Extracted
// from useTurnNav (session-detail-sheet.tsx) so the routing invariant is
// testable (architecture.md: "关键不变量用代码表达"). The hook wires this to
// react-virtuoso's rangeChanged / scrollToIndex + the flash side-effects; the
// math lives here.

import type { SessionMessage } from "@/types/generated/bindings"

/** A user turn's slot in the transcript: its index in the message array + the
 *  user message's uuid. rangeChanged and scrollToIndex speak in message
 *  indices, so the index must travel with the turn. */
export interface TurnAnchor {
  index: number
  uuid: string
}

/** The ordered user-turn anchors for a transcript (user messages with their
 *  indices). */
export function turnAnchors(messages: readonly SessionMessage[]): TurnAnchor[] {
  const out: TurnAnchor[] = []
  for (let i = 0; i < messages.length; i++) {
    if (messages[i].role === "user")
      out.push({ index: i, uuid: messages[i].uuid })
  }
  return out
}

/** The user-turn anchor at or before a message index, else null (the index sits
 *  above the first turn). Shared by the public turnAtAnchor and the reducer's
 *  boundary checks — both need the anchor's index, not just its uuid. */
function anchorAtOrBefore(
  turns: readonly TurnAnchor[],
  index: number,
): TurnAnchor | null {
  let res: TurnAnchor | null = null
  for (const t of turns) {
    if (t.index <= index) res = t
    else break
  }
  return res
}

/** The active turn at a scroll anchor = the last user turn at or above the
 *  anchor (the turn whose content starts at or above the viewport top). null
 *  when the anchor sits above the first turn. */
export function turnAtAnchor(
  turns: readonly TurnAnchor[],
  anchor: number,
): string | null {
  return anchorAtOrBefore(turns, anchor)?.uuid ?? null
}

/** The message index of the turn immediately before `index`, or null when
 *  `index` is the first turn (no turn precedes it). */
function indexBefore(
  turns: readonly TurnAnchor[],
  index: number,
): number | null {
  let prevIdx: number | null = null
  for (const t of turns) {
    if (t.index >= index) break
    prevIdx = t.index
  }
  return prevIdx
}

/** Routing state: the active turn + the turn a jump has pinned. A jump pins its
 *  target turn and holds it through the scroll burst that follows; the pin
 *  clears once the user genuinely navigates to another turn. */
export interface TurnNavState {
  activeUuid: string | null
  /** The pinned turn while the user reads around a jump target, else null. */
  pin: TurnAnchor | null
}

export const initialTurnNav: TurnNavState = { activeUuid: null, pin: null }

export type TurnNavEvent =
  | { type: "jump"; targetIndex: number }
  | { type: "range"; startIndex: number }

/**
 * Reduce one routing event. Pure: same (state, event, turns) → same next state.
 *
 * Why a pin instead of skipping the next rangeChanged: a jump parks its target
 * TURN_OFFSET below the viewport top (so it isn't flush against the header), so
 * the post-jump rangeChanged reports the PREVIOUS turn's content at the top —
 * and react-virtuoso fires a BURST of them as a long scroll settles (scroll
 * steps + dynamic-height re-measurement). Skipping just the first left the
 * second event free to retreat the highlight to the previous turn (the "click
 * turn 14, highlight turn 13" bug). The pin holds the target through the whole
 * burst and only hands control back to scroll tracking once the user crosses a
 * neighbouring turn boundary — forward into the next turn, or back past the
 * turn immediately before the pin. Minor scrolls that stay inside the pin's own
 * span (or the one just before it) keep the pin, which is the intended "the
 * turn you jumped to stays active while you read around it" feel.
 */
export function reduceTurnNav(
  state: TurnNavState,
  event: TurnNavEvent,
  turns: readonly TurnAnchor[],
): TurnNavState {
  if (event.type === "jump") {
    const owner = anchorAtOrBefore(turns, event.targetIndex)
    return { activeUuid: owner?.uuid ?? null, pin: owner }
  }
  // range
  const candidate = anchorAtOrBefore(turns, event.startIndex)
  if (state.pin === null) {
    const uuid = candidate?.uuid ?? null
    return state.activeUuid === uuid ? state : { activeUuid: uuid, pin: null }
  }
  const pinIdx = state.pin.index
  const candidateIdx = candidate?.index ?? -1
  if (candidateIdx > pinIdx) {
    // scrolled forward into a later turn → advance and release the pin
    return { activeUuid: candidate?.uuid ?? null, pin: null }
  }
  // candidate is the pin itself or an earlier turn. Release on a genuine
  // retreat (scrolled up past the turn immediately before the pin); otherwise
  // hold — the settling burst and small upward scrolls report the previous
  // turn's span, which is exactly the case the pin exists to absorb.
  const prevIdx = indexBefore(turns, pinIdx)
  if (prevIdx !== null && candidateIdx < prevIdx) {
    return { activeUuid: candidate?.uuid ?? null, pin: null }
  }
  return state
}
