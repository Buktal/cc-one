import { useEffect, useState } from "react"

/** Track a CSS media query (window-level). SSR-safe-ish: until the effect
 *  runs, the initial state is the matchMedia result (or false when absent —
 *  vitest's node environment). Re-renders on change; listeners cleaned up.
 *
 *  Used for the shell's < 1024px auto-collapse (the four-column workbench's
 *  主导航 breakpoint) — container queries can't express "the window is
 *  narrow" for an element outside the measured container. */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => {
    if (typeof window === "undefined" || !window.matchMedia) return false
    return window.matchMedia(query).matches
  })
  useEffect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return
    const mql = window.matchMedia(query)
    const onChange = (e: MediaQueryListEvent) => setMatches(e.matches)
    setMatches(mql.matches)
    mql.addEventListener("change", onChange)
    return () => mql.removeEventListener("change", onChange)
  }, [query])
  return matches
}
