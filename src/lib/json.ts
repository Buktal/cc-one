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

/** 容错 parse JSON 文本为对象：空串 / 语法错误 / 非对象顶层（数组、标量）→
 *  `undefined`，调用方归一为自己的默认值。与 `parseJsonObject` 的严格判错
 *  契约相对——这是 provider codec 族「吞垃圾归默认」（坏快照不崩表单）的
 *  共享骨架：claude/codex/gemini/grok/opencode 的 parse* 与 derive 的
 *  parseMeta 都经此，容错语义只有一份实现。注意 `snippet.ts` 的
 *  `parseSnippetInput` 不用它——那里的契约刻意严格（空与垃圾必须分得清，
 *  垃圾配置不能误报「片段将补 X」），与「吞垃圾」的宽容版不可混用。 */
export function parseJsonObjectLenient(
  text: string,
): Record<string, unknown> | undefined {
  if (!text) return undefined
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return undefined
    }
    return parsed as Record<string, unknown>
  } catch {
    return undefined
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

/** 把对象键按字母序重排（就地返回新对象，不改原值）。只排顶层 + env 内键
 *  （ADR-0011：「JSON 片段按顶层 + env 内键排序」）——env 是设置键的扁平 map，
 *  排序有意义；更深的嵌套对象（如 mcpServers 配置）键序保持用户原样，不递归
 *  重排。数组元素保持原序。 */
function sortKeysShallow(value: unknown): unknown {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return value
  }
  const out: Record<string, unknown> = {}
  for (const key of Object.keys(value).sort()) {
    let v = (value as Record<string, unknown>)[key]
    if (key === "env") {
      v = sortKeysShallow(v)
    }
    out[key] = v
  }
  return out
}

/** 整理 JSON 片段（「整理」按钮）：格式化 + 键字母序排序（顶层 + env 内键，
 *  ADR-0011）。先容错排版（jsonc-parser，输入可能是 jsonc / 尾逗号），再尝试
 *  严格解析——若合法则排序键 + stringify；若非法（含注释等）回退到 formatJson
 *  结果（只排版不排序，绝不因排序失败而抛错）。幂等。 */
export function tidyJson(text: string): string {
  const formatted = formatJson(text)
  try {
    const parsed: unknown = JSON.parse(formatted)
    return JSON.stringify(sortKeysShallow(parsed), null, 2)
  } catch {
    return formatted
  }
}
