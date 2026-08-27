// Tests for the transcript view's presentation derivations (architecture.md:
// "测试必须跑生产路径"). The row open/collapse xor rule, the bulk collapse /
// expand end states, transcript-wide search snippets, and the one-line display
// shorthands — every function in ./transcript is pure, so these are
// table-driven unit cases run in vitest's node-only environment (no DOM — see
// vitest.config.ts).

import { describe, expect, it } from "vitest"

import type { SessionMessage } from "@/types/generated/bindings"

import {
  collapseAllMessages,
  expandAllMessages,
  firstLine,
  isAllCollapsed,
  isRowOpen,
  roleDefaultsCollapsed,
  transcriptMatches,
  tryFormatJson,
} from "./transcript"

describe("firstLine", () => {
  it("returns the first line of multiline text", () => {
    expect(firstLine("hello\nworld\n")).toBe("hello")
  })

  it("single-line text returns itself", () => {
    expect(firstLine("solo")).toBe("solo")
  })

  it("empty text yields an empty string", () => {
    expect(firstLine("")).toBe("")
  })
})

describe("tryFormatJson", () => {
  it("pretty-prints an object with 2-space indent", () => {
    expect(tryFormatJson('{"a":1,"b":[2,3]}')).toBe(
      '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ]\n}',
    )
  })

  it("pretty-prints an array", () => {
    expect(tryFormatJson('[1,"x"]')).toBe('[\n  1,\n  "x"\n]')
  })

  it("rejects plain text", () => {
    expect(tryFormatJson("not json at all")).toBeNull()
  })

  it("rejects malformed json", () => {
    expect(tryFormatJson('{"a":')).toBeNull()
  })

  it("rejects scalar json (a bare string or number formats to nothing)", () => {
    expect(tryFormatJson('"just a string"')).toBeNull()
    expect(tryFormatJson("42")).toBeNull()
  })
})

describe("collapseAllMessages / expandAllMessages / isAllCollapsed", () => {
  const msgs = (...roles: SessionMessage["role"][]): SessionMessage[] =>
    roles.map((role, i) => ({
      uuid: `u${i}`,
      role,
      content: "",
    })) as SessionMessage[]

  it("collapseAll puts every non-tool row in the set", () => {
    expect(
      collapseAllMessages(msgs("user", "assistant", "tool", "system")),
    ).toEqual(new Set(["u0", "u1", "u3"]))
  })

  it("expandAll puts every tool row in the set", () => {
    expect(
      expandAllMessages(msgs("user", "assistant", "tool", "system")),
    ).toEqual(new Set(["u2"]))
  })

  it("isAllCollapsed is true only when every non-tool row is in the set", () => {
    const all = msgs("user", "assistant", "tool", "system")
    expect(isAllCollapsed(all, new Set(["u0", "u1", "u3"]))).toBe(true)
    expect(isAllCollapsed(all, new Set(["u0", "u3"]))).toBe(false)
  })

  it("isAllCollapsed is false on a tool-only or empty transcript", () => {
    expect(isAllCollapsed(msgs("tool"), new Set())).toBe(false)
    expect(isAllCollapsed([], new Set())).toBe(false)
  })

  it("bulk sets round-trip through the detail view's real isRowOpen", () => {
    // The detail view's per-row open state runs the same xor rule as the bulk
    // sets (single source in transcript): after collapseAll no row is open,
    // after expandAll every row is open. Calls the production isRowOpen — no
    // fork (architecture.md: "测试必须跑生产路径").
    const all = msgs("user", "tool", "assistant")
    const collapsed = collapseAllMessages(all)
    expect(all.every((m) => !isRowOpen(m.uuid, m.role, collapsed))).toBe(true)
    const expanded = expandAllMessages(all)
    expect(all.every((m) => isRowOpen(m.uuid, m.role, expanded))).toBe(true)
  })

  it("read/write consistency: every role's default matches both ends", () => {
    // The xor rule has one source (roleDefaultsCollapsed). A third
    // default-collapsed role would have to change that predicate, and this
    // table forces the change there — not silently in two files.
    for (const role of ["user", "assistant", "tool", "system"] as const) {
      const m = msgs(role)[0]
      const collapsed = collapseAllMessages([m])
      const expanded = expandAllMessages([m])
      expect(
        isRowOpen(m.uuid, m.role, collapsed),
        `${role} row must be closed after collapseAll`,
      ).toBe(false)
      expect(
        isRowOpen(m.uuid, m.role, expanded),
        `${role} row must be open after expandAll`,
      ).toBe(true)
    }
  })
})

describe("roleDefaultsCollapsed", () => {
  it("only tool rows default collapsed", () => {
    expect(roleDefaultsCollapsed("tool")).toBe(true)
    expect(roleDefaultsCollapsed("user")).toBe(false)
    expect(roleDefaultsCollapsed("assistant")).toBe(false)
    expect(roleDefaultsCollapsed("system")).toBe(false)
  })
})

describe("transcriptMatches", () => {
  let seq = 0
  const msg = (
    content: string,
    ts = "2026-08-12T10:00:00Z",
  ): SessionMessage => {
    seq += 1
    return {
      uuid: `u${seq}`,
      role: "assistant",
      content,
      ts,
      session_id: "s",
    } as SessionMessage
  }

  it("empty / whitespace query → no hits", () => {
    expect(transcriptMatches([msg("hello world")], "")).toEqual([])
    expect(transcriptMatches([msg("hello world")], "   ")).toEqual([])
  })

  it("no match → no hits", () => {
    expect(transcriptMatches([msg("hello world")], "nope")).toEqual([])
  })

  it("matches are case-insensitive and keep transcript order", () => {
    const ms = [
      msg("Fix the BUG here"),
      msg("another one"),
      msg("no bug at all"),
    ]
    expect(transcriptMatches(ms, "bug").map((h) => h.message)).toEqual([
      ms[0],
      ms[2],
    ])
  })

  it("snippet keeps the hit intact for the renderer to highlight", () => {
    const [hit] = transcriptMatches(
      [msg("prefix 1234567890 bug 1234567890 suffix")],
      "bug",
    )
    expect(hit.snippet).toContain("bug")
  })

  it("snippet ellipsizes both edges when the hit sits mid-text", () => {
    const text = `${"x".repeat(100)} needle ${"y".repeat(100)}`
    const [hit] = transcriptMatches([msg(text)], "needle")
    expect(hit.snippet.startsWith("…")).toBe(true)
    expect(hit.snippet.endsWith("…")).toBe(true)
    // RADIUS (28) both sides + the 6-char hit + 2 ellipses.
    expect(hit.snippet).toHaveLength(28 + 6 + 28 + 2)
  })

  it("snippet at the start of the text has no leading ellipsis", () => {
    const text = `needle ${"y".repeat(100)}`
    const [hit] = transcriptMatches([msg(text)], "needle")
    expect(hit.snippet.startsWith("needle")).toBe(true)
    expect(hit.snippet.endsWith("…")).toBe(true)
  })

  it("snippet at the end of the text has no trailing ellipsis", () => {
    const text = `${"x".repeat(100)} needle`
    const [hit] = transcriptMatches([msg(text)], "needle")
    expect(hit.snippet.endsWith("needle")).toBe(true)
  })
})
