import type { TFunction } from "i18next"
import { describe, expect, it } from "vitest"

import {
  describeError,
  localizeStructuredError,
  rawErrorText,
  toStructuredError,
} from "@/lib/error"

// A stand-in for i18next's `t`: echoes the key, appending the interpolated
// `data` so tests can assert both the chosen key and the payload. Cast to
// `TFunction` (a branded type) so the signature matches production callers.
const t = ((key: string, opts?: Record<string, unknown>) =>
  opts && "data" in opts ? `${key}:${String(opts.data)}` : key) as TFunction

describe("describeError", () => {
  it("maps a structured AppError to errors.<type> with data interpolation", () => {
    expect(describeError({ type: "Config", data: "bad token" }, t)).toBe(
      "errors.Config:bad token",
    )
    expect(describeError({ type: "Sync", data: "offline" }, t)).toBe(
      "errors.Sync:offline",
    )
    expect(describeError({ type: "Internal", data: "serde: eof" }, t)).toBe(
      "errors.Internal:serde: eof",
    )
  })

  it("covers every backend AppError variant", () => {
    const variants = [
      "Config",
      "Db",
      "SourceParser",
      "Pricing",
      "Sync",
      "Internal",
    ]
    for (const type of variants) {
      expect(describeError({ type, data: "x" }, t)).toBe(`errors.${type}:x`)
    }
  })

  it("extracts the message from a thrown Error (non-API path, e.g. updater)", () => {
    expect(describeError(new Error("boom"), t)).toBe("boom")
  })

  it("extracts .message from a plain object (RTK Query-serialised shape)", () => {
    expect(describeError({ message: "network down" }, t)).toBe("network down")
  })

  it("extracts a string .data / .error field when there is no .message", () => {
    expect(describeError({ data: "rate limited" }, t)).toBe("rate limited")
    expect(describeError({ error: "denied" }, t)).toBe("denied")
  })

  it("returns a bare string verbatim", () => {
    expect(describeError("plain string", t)).toBe("plain string")
  })

  it("returns empty when nothing recognisable (caller adds fallback)", () => {
    expect(describeError(null, t)).toBe("")
    expect(describeError(undefined, t)).toBe("")
    expect(describeError({}, t)).toBe("")
    expect(describeError(42, t)).toBe("")
  })
})

describe("toStructuredError", () => {
  it("keeps a backend AppError as the re-translatable app shape", () => {
    expect(toStructuredError({ type: "Config", data: "bad token" })).toEqual({
      kind: "app",
      type: "Config",
      data: "bad token",
    })
  })

  it("collapses a thrown Error to its raw message", () => {
    expect(toStructuredError(new Error("boom"))).toEqual({
      kind: "raw",
      message: "boom",
    })
  })

  it("collapses a plain object / bare string to a raw message", () => {
    expect(toStructuredError({ message: "net down" })).toEqual({
      kind: "raw",
      message: "net down",
    })
    expect(toStructuredError("oops")).toEqual({ kind: "raw", message: "oops" })
  })

  it("returns null when nothing recognisable (no translation to defer)", () => {
    expect(toStructuredError(null)).toBeNull()
    expect(toStructuredError({})).toBeNull()
    expect(toStructuredError(42)).toBeNull()
  })
})

describe("rawErrorText (mutation-error display chain)", () => {
  // Production path: provider-form-sheet / live-import-dialog /
  // cc-switch-import-dialog surface mutation errors through this seam.

  it("prefers the AppError data (the backend's message payload)", () => {
    expect(rawErrorText({ type: "Config", data: "bad token" })).toBe(
      "bad token",
    )
  })

  it("falls back to the raw message of a thrown Error / serialised shape", () => {
    expect(rawErrorText(new Error("boom"))).toBe("boom")
    expect(rawErrorText({ message: "network down" })).toBe("network down")
    expect(rawErrorText({ data: "rate limited" })).toBe("rate limited")
    expect(rawErrorText({ error: "denied" })).toBe("denied")
  })

  it("passes a bare string through verbatim", () => {
    expect(rawErrorText("plain string")).toBe("plain string")
  })

  it("never returns empty — String() is the last-resort fallback", () => {
    expect(rawErrorText(null)).toBe("null")
    expect(rawErrorText(undefined)).toBe("undefined")
    expect(rawErrorText(42)).toBe("42")
    expect(rawErrorText({})).toBe("[object Object]")
  })
})

describe("localizeStructuredError", () => {
  it("translates the app shape via errors.<type> with data", () => {
    expect(
      localizeStructuredError(
        { kind: "app", type: "Sync", data: "offline" },
        t,
      ),
    ).toBe("errors.Sync:offline")
  })

  it("passes the raw shape through unchanged (no translation)", () => {
    expect(localizeStructuredError({ kind: "raw", message: "boom" }, t)).toBe(
      "boom",
    )
  })
})
