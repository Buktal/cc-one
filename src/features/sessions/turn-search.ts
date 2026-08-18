// Turn-nav in-session search state — the "when does search mode exit / when is
// the hit highlight kept" decision for the TurnNavPanel, as a pure reducer.
// Extracted from TurnNavPanel (session-detail-sheet.tsx) so the exit semantics
// are an assertable state machine (architecture.md: "关键不变量用代码表达").
// The panel wires this to the toolbar toggle / input / Esc / clear-button /
// hit-click handlers; the transitions live here.

/** In-session search state for the turn-nav panel. */
export interface TurnSearchState {
  /** Search mode active — the panel shows the search UI instead of the turn index. */
  searching: boolean
  /** The live query, fed straight through from the input. */
  query: string
  /** The search hit most recently jumped to — kept highlighted until the
   *  query changes, so the eye has an anchor while reading the transcript. */
  lastJumped: string | null
}

export const initialTurnSearch: TurnSearchState = {
  searching: false,
  query: "",
  lastJumped: null,
}

export type TurnSearchEvent =
  /** The toolbar button — enters search mode, or exits it when already in. */
  | { type: "toggle" }
  /** Esc in the search input — exits. */
  | { type: "esc" }
  /** The clear button (shown while the query is non-empty) — exits. */
  | { type: "clear" }
  /** The input changed — re-scan with the new query, drop the highlight. */
  | { type: "query"; query: string }
  /** A search hit was clicked — keep it highlighted. */
  | { type: "hit"; uuid: string }

/**
 * Reduce one search event. Pure: same (state, event) → same next state.
 *
 * The exit invariant: every way out of search mode — toolbar toggle while in,
 * Esc, the clear button — lands on the clean state (empty query, no
 * highlight), and a query change drops the highlight. The panel therefore
 * never shows a stale highlight for a query it no longer matches, and never
 * reopens with a leftover query.
 */
export function reduceTurnSearch(
  state: TurnSearchState,
  event: TurnSearchEvent,
): TurnSearchState {
  switch (event.type) {
    case "toggle":
      // Entering keeps whatever the state is — which is always clean, because
      // the exits below reset it. Exiting drops the query + highlight.
      return state.searching
        ? { searching: false, query: "", lastJumped: null }
        : { ...state, searching: true }
    case "esc":
    case "clear":
      return { searching: false, query: "", lastJumped: null }
    case "query":
      return { ...state, query: event.query, lastJumped: null }
    case "hit":
      return { ...state, lastJumped: event.uuid }
  }
}
