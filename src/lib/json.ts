// Generic JSON helpers shared by the JSON editor and the provider form sheet's
// settingsConfig sync. Kept in lib/ (not a feature) so the editor stays
// feature-agnostic — any future settings-snapshot editor can reuse them.

import { applyEdits, format } from "jsonc-parser"

/** Result of parsing JSON text into a plain object. Empty text counts as `{}`
 *  (a blank snapshot); a syntax error and a non-object top level are reported
 *  separately so callers can decide which failure to surface. */
export type JsonObjectResult =
  | { ok: true; value: Record<string, unknown> }
  | { ok: false; error: string }

/** Parse JSON text into a plain object, tagging the outcome. The top-level
 *  must be an object — a settings snapshot that parses to an array or a bare
 *  string is a corrupt snapshot, not a valid config. */
export function parseJsonObject(text: string): JsonObjectResult {
  const trimmed = text.trim()
  if (!trimmed) return { ok: true, value: {} }
  try {
    const parsed: unknown = JSON.parse(trimmed)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { ok: false, error: "Expected a JSON object" }
    }
    return { ok: true, value: parsed as Record<string, unknown> }
  } catch (err) {
    return {
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    }
  }
}

/** 容错格式化：与 VS Code 的 JSON 编辑器同款（jsonc-parser tokenizer 重排，
 *  不依赖完整 parse）——2 空格缩进展开成多行；语法错误 / 注释 / 尾逗号等
 *  无效内容也能展开成可读结构（字符串字面量里的逗号、括号不受影响）。
 *  不抛错；已展开的文本幂等（重复调用不变）。合法 JSON 的输出与
 *  JSON.stringify(parsed, null, 2) 一致。 */
export function formatJson(text: string): string {
  const trimmed = text.trim()
  const edits = format(trimmed, undefined, { tabSize: 2, insertSpaces: true })
  return applyEdits(trimmed, edits)
}
