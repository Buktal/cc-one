import { useEffect, useState } from "react"

/**
 * Return `value` after `delayMs` of quiet — for keystroke-driven backend
 * filters (the sessions search box) so every keystroke doesn't fire a query
 * while typing. The latest value wins; a stale timer's output is discarded.
 */
export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delayMs)
    return () => clearTimeout(id)
  }, [value, delayMs])
  return debounced
}
