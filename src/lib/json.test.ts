// Tests for the generic JSON helpers shared by the JSON editor and the
// provider form sheet's settingsConfig sync (lib/json.ts).

import { describe, expect, it } from "vitest"
import { formatJson, parseJsonObject, tidyJson } from "@/lib/json"

describe("parseJsonObject", () => {
  it("parses a plain object", () => {
    const result = parseJsonObject('{"env": {"ANTHROPIC_MODEL": "m"}}')
    expect(result).toEqual({
      ok: true,
      value: { env: { ANTHROPIC_MODEL: "m" } },
    })
  })

  it("treats empty text as an empty object (a blank snapshot)", () => {
    expect(parseJsonObject("")).toEqual({ ok: true, value: {} })
    expect(parseJsonObject("   ")).toEqual({ ok: true, value: {} })
  })

  it("flags a syntax error without throwing", () => {
    const result = parseJsonObject('{"env": ')
    expect(result.ok).toBe(false)
    if (!result.ok) expect(result.error.length).toBeGreaterThan(0)
  })

  it("flags a non-object top level (array / string / number)", () => {
    expect(parseJsonObject("[1, 2]").ok).toBe(false)
    expect(parseJsonObject('"a bare string"').ok).toBe(false)
    expect(parseJsonObject("42").ok).toBe(false)
    expect(parseJsonObject("null").ok).toBe(false)
  })
})

describe("formatJson", () => {
  it("trims, parses and stringifies with 2-space indentation", () => {
    expect(formatJson('  {"b":1,"a":[1,2]}  ')).toBe(
      '{\n  "b": 1,\n  "a": [\n    1,\n    2\n  ]\n}',
    )
  })

  it("leaves already-formatted JSON unchanged (idempotent)", () => {
    const text = '{\n  "env": {}\n}'
    expect(formatJson(text)).toBe(text)
  })

  it("spreads broken JSON into a readable outline without throwing", () => {
    expect(formatJson('{"a":1,"b":')).toBe('{\n  "a": 1,\n  "b":')
  })

  it("keeps string literals intact (commas / braces inside strings)", () => {
    expect(formatJson('{"msg":"a,b{c}","n":1')).toBe(
      '{\n  "msg": "a,b{c}",\n  "n": 1',
    )
  })

  it("handles JSONC comments and trailing commas", () => {
    expect(formatJson('// note\n{"a":1,}')).toBe('// note\n{\n  "a": 1,\n}')
  })

  it("returns empty string for empty input", () => {
    expect(formatJson("")).toBe("")
  })
})

describe("tidyJson", () => {
  it("sorts top-level keys and env keys alphabetically (ADR-0011)", () => {
    expect(
      tidyJson(
        '{"includeCoAuthoredBy":false,"env":{"GEMINI_MODEL":"m","GEMINI_API_KEY":"k"}}',
      ),
    ).toBe(
      '{\n  "env": {\n    "GEMINI_API_KEY": "k",\n    "GEMINI_MODEL": "m"\n  },\n  "includeCoAuthoredBy": false\n}',
    )
  })

  it("does not reorder deeper nested objects (only top level + env)", () => {
    // ADR-0011 只排「顶层 + env 内键」——深层对象键序保持用户原样。
    const text = '{"mcpServers":{"z":{"command":"npx"},"a":{"command":"ls"}}}'
    expect(tidyJson(text)).toContain(
      '"mcpServers": {\n    "z": {\n      "command": "npx"\n    },\n    "a": {',
    )
  })

  it("falls back to layout-only for invalid JSON (never throws)", () => {
    expect(tidyJson('{"a":1,')).toBe('{\n  "a": 1,')
  })

  it("is idempotent", () => {
    const once = tidyJson('{"b":1,"a":2}')
    expect(tidyJson(once)).toBe(once)
  })
})
