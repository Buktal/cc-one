// Display formatting helpers. The JS layer never computes
// cost — these are display-only shapers for numbers, currency, dates.
//
// Locale policy: token counts are ALWAYS K/M/B (international-neutral,
// language-independent); cost is always USD `$`; dates are always the compact
// numeric `MM/DD HH:mm`. Only the relative-time words (`fromNow`) follow the
// language — driven by the dayjs locale set in `@/i18n/languages`. So nothing
// here hard-codes a dayjs locale.

import dayjs from "dayjs"

/** Compact a token count to K/M/B: `3.61M`, `1.2B`, `856`. Language-independent. */
export function formatTokens(n: number | null | undefined): string {
  const v = Number(n ?? 0)
  if (!Number.isFinite(v)) return "0"
  if (v >= 1e9) return `${trim(v / 1e9)}B`
  if (v >= 1e6) return `${trim(v / 1e6)}M`
  if (v >= 1e3) return `${trim(v / 1e3)}K`
  return v.toLocaleString("en-US")
}

/** USD amount with 4 decimals, no currency symbol — `1.7564`. Null/0 →
 *  `0.0000`. Tables that carry the `$` unit in the column header use this so
 *  the symbol doesn't repeat per cell. */
export function formatCostAmount(usd: number | null | undefined): string {
  const v = Number(usd ?? 0)
  if (!Number.isFinite(v)) return "0.0000"
  return v.toFixed(4)
}

/** USD cost with 4 decimals, e.g. `$1.7564`. Null/0 → `$0.0000`. */
export function formatCost(usd: number | null | undefined): string {
  return `$${formatCostAmount(usd)}`
}

/** Integer with thousands separators. */
export function formatInt(n: number | null | undefined): string {
  const v = Math.trunc(Number(n ?? 0))
  return v.toLocaleString("en-US")
}

/** Ratio in [0,1] → percent string `90.2%`. */
export function formatPct(rate: number | null | undefined): string {
  const v = Number(rate ?? 0)
  if (!Number.isFinite(v)) return "0%"
  return `${(v * 100).toFixed(1)}%`
}

/** Milliseconds → `12.3s` / `1m05s`. Em-dash when absent / non-positive. */
export function formatDuration(ms: number | null | undefined): string {
  const v = Number(ms ?? 0)
  if (!Number.isFinite(v) || v <= 0) return "—"
  if (v < 60_000) return `${(v / 1000).toFixed(1)}s`
  const m = Math.floor(v / 60_000)
  const sec = Math.round((v % 60_000) / 1000)
  return `${m}m${sec.toString().padStart(2, "0")}s`
}

/** Bytes → `1.2 KB` / `3.4 MB` / `5.67 GB`. Em-dash when absent / non-finite. */
export function formatSize(bytes: number | null | undefined): string {
  const v = Number(bytes ?? 0)
  if (!Number.isFinite(v) || v <= 0) return "—"
  if (v < 1024) return `${Math.round(v)} B`
  if (v < 1024 ** 2) return `${(v / 1024).toFixed(1)} KB`
  if (v < 1024 ** 3) return `${(v / 1024 ** 2).toFixed(1)} MB`
  return `${(v / 1024 ** 3).toFixed(2)} GB`
}

/** ISO timestamp → `MM/DD HH:mm`. Falls back to the raw string on bad input. */
export function formatTime(ts: string | null | undefined): string {
  if (!ts) return "—"
  const d = dayjs(ts)
  return d.isValid() ? d.format("MM/DD HH:mm") : ts
}

/** ISO day `yyyy-mm-dd` → `MM/DD`. */
export function formatDay(day: string | null | undefined): string {
  if (!day) return "—"
  const d = dayjs(day)
  return d.isValid() ? d.format("MM/DD") : day
}

/** Convert a `<input type="date">` value (yyyy-mm-dd) to a filter day or null. */
export function dateInputToDay(v: string): string | null {
  return v && v.trim() !== "" ? v.trim() : null
}

function trim(n: number): string {
  // 2 decimals, drop trailing zeros for compactness.
  return n
    .toFixed(2)
    .replace(/\.?0+$/, "")
    .trim()
}
