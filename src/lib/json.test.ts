// Tests for the generic JSON helpers shared by the JSON editor and the
// provider form sheet's settingsConfig sync (lib/json.ts).

import { describe, expect, it } from "vitest"
import { formatJson, parseJsonObject } from "@/lib/json"

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
