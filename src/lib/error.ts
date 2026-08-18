// Single error-message seam. Two layers:
//   - `toStructuredError(e)` — pure reduction of any error shape to a
//     `StructuredError` with no translation and no i18n dependency. The
//     backend's `{ type, data }` AppError (returned, not thrown — see
//     `run`/`queryFn` in api.ts) is kept as the `app` shape so it stays
//     re-translatable; every other shape (a thrown JS `Error` from the updater
//     plugin, an RTK-Query-serialised `{ message }`, a raw string) collapses to
//     its raw message.
//   - `localizeStructuredError(s, t)` — translates a `StructuredError` at the
//     render boundary (`app` → `errors.<type>` key with `data`; `raw` → as-is).
// State that crosses a Redux boundary (e.g. updateSlice) keeps the structured
// form and localises at render, so a language switch re-translates the reason
// instead of freezing the old-language string. `describeError(e, t)` composes
// the two for call sites that localise immediately (returned, not stored).
// Returns "" / null when nothing recognisable is found, so callers compose
// their own fallback: `describeError(e, t) || t("common.unknownReason")`.

import type { TFunction } from "i18next"

/** Structural guard for the backend's discriminated error (`{ type, data }`). */
function isAppError(e: unknown): e is { type: string; data: string } {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as Record<string, unknown>).type === "string" &&
    typeof (e as Record<string, unknown>).data === "string"
  )
}

/** A failure reason in structured form — localizable at the render boundary,
 *  not before dispatch. The `app` shape carries the backend's `{ type, data }`
 *  discriminator (re-translatable on a language switch); the `raw` shape is an
 *  already-final string (a thrown `Error.message`, a source-parser error string) with no
 *  translation to apply. */
export type StructuredError =
  | { kind: "app"; type: string; data: string }
  | { kind: "raw"; message: string }

/**
 * Reduce any error shape to its structured form — pure, no translation. The
 * `app` shape preserves the discriminator so a later `localizeStructuredError`
 * can re-translate on a language switch; every other shape collapses to its
 * raw message. Returns null when nothing recognisable is present (callers
 * compose their own fallback).
 */
export function toStructuredError(e: unknown): StructuredError | null {
  if (isAppError(e)) return { kind: "app", type: e.type, data: e.data }
  const message = rawErrorMessage(e)
  return message ? { kind: "raw", message } : null
}

/** Extract a raw, already-final message from a non-AppError shape: a thrown
 *  `Error.message`, a plain object's `.message` / `.data` / `.error`, or a bare
 *  string. "" when nothing recognisable. */
function rawErrorMessage(e: unknown): string {
  if (e instanceof Error) return e.message
  if (e && typeof e === "object") {
    const m = e as Record<string, unknown>
    if (typeof m.message === "string") return m.message
    if (typeof m.data === "string") return m.data
    if (typeof m.error === "string") return m.error
  }
  return typeof e === "string" ? e : ""
}

/** Extract the raw, already-final text of an error for display: an AppError's
 *  `data` (the backend's message payload) when the error is structured,
 *  otherwise the raw message, with `String(e)` as a last resort. Unlike
 *  `describeError` / `toStructuredError` (which return "" / null and leave the
 *  fallback to the caller), this is the shared fallback chain of the mutation
 *  error paths — never "". Three copies of this chain (provider-form-sheet /
 *  live-import-dialog / cc-switch-import-dialog) converge here. */
export function rawErrorText(e: unknown): string {
  const s = toStructuredError(e)
  return s?.kind === "app" ? s.data : (s?.message ?? String(e))
}

/** Translate a structured error at the render boundary. `app` → the matching
 *  `errors.<type>` i18n key (with `data` interpolation); `raw` → the string
 *  unchanged. */
export function localizeStructuredError(
  s: StructuredError,
  t: TFunction,
): string {
  return s.kind === "app" ? t(`errors.${s.type}`, { data: s.data }) : s.message
}

/** Extract a readable, translated reason from an unknown error — convenience
 *  for call sites that localise immediately instead of storing the structured
 *  form across a boundary. Composes `toStructuredError` +
 *  `localizeStructuredError`; "" when nothing recognisable. */
export function describeError(e: unknown, t: TFunction): string {
  const s = toStructuredError(e)
  return s ? localizeStructuredError(s, t) : ""
}
