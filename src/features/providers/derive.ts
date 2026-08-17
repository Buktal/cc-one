// 供应商表单派生逻辑的聚合层（架构扫描候选⑤拆分后）：各应用的 settingsConfig
// codec 在 codecs/（claude/codex/gemini/grok/opencode.ts，各自 parse/with 家族）、
// 片段域在 snippet.ts、必填检查在 missing.ts。本文件保留跨应用的「表单聚合」
// 逻辑——模板变量（preset 的 `${VAR}` placeholder）、app 侧 meta 记录、列表
// 过滤与新建/预设草稿——并 `export *` 重导出各域符号，保持既有调用方的
// `from "@/features/providers/derive"` import 不变。改单个 app 的 codec 不再
// 需要打开这个聚合文件。

import type { ProviderPreset } from "@/features/providers/presets"
import type { App, Provider } from "@/types/generated/bindings"
import { parseSettingsConfig } from "./codecs/claude"

export * from "./codecs/claude"
export * from "./codecs/codex"
export * from "./codecs/gemini"
export * from "./codecs/grok"
export * from "./codecs/opencode"
export * from "./missing"
export * from "./snippet"

/** 按名称（不区分大小写，contains）或分类标识过滤供应商；空查询 → 原列表。
 *  纯函数供列表搜索框用（可测试），分类按 `providers.category.<id>` 的
 *  i18n 键对应的标识匹配。 */
export function filterProviders(
  providers: Provider[],
  query: string,
): Provider[] {
  const q = query.trim().toLowerCase()
  if (!q) return providers
  return providers.filter(
    (p) =>
      p.name.toLowerCase().includes(q) || p.category.toLowerCase().includes(q),
  )
}

/** A blank provider for the "new provider" sheet (custom category, empty
 *  env/config — the form starts fresh). `app` defaults to claude (the original
 *  pool); the settingsConfig shape matches the app: claude = `{"env": {}}`,
 *  codex / gemini = `"{}"` (both tolerate empty in their parsers). `id` is
 *  empty so `save_provider_cmd` allocates a fresh one. */
export function emptyProvider(app: App = "claude"): Provider {
  return {
    id: "",
    name: "",
    websiteUrl: "",
    category: "custom",
    app,
    icon: "",
    iconColor: "",
    sortIndex: 0,
    notes: "",
    settingsConfig: app === "claude" ? '{\n  "env": {}\n}' : "{}",
    meta: "{}",
    updatedAt: "",
  }
}

/** Build the "new provider" draft from a built-in preset: category keeps the
 *  preset's own (`cloud_provider` / `aggregator` stay 云厂商 / 聚合——列表分类
 *  与切换检查按它区分，抹成 custom 会让云端供应商被误要求填端点/key), `id`
 *  stays empty so `save_provider_cmd` allocates a fresh one. The preset's
 *  settingsConfig snapshot is copied verbatim (its `${VAR}` placeholders stay
 *  until the template-variable step); the preset constant itself is never
 *  mutated. `app` defaults to claude — the preset arrays are app-scoped (see
 *  `presetsForApp`), so the caller knows which pool the preset came from. */
export function providerFromPreset(
  preset: ProviderPreset,
  app: App = "claude",
): Provider {
  return {
    id: "",
    name: preset.name,
    websiteUrl: preset.websiteUrl,
    category: preset.category,
    app,
    icon: preset.icon,
    iconColor: preset.iconColor,
    sortIndex: 0,
    notes: preset.notes ?? "",
    settingsConfig: preset.settingsConfig,
    meta: "{}",
    updatedAt: "",
  }
}

// ── Template variables ─────────────────────────────────────────────────────
//
// Some presets (Bedrock) carry `${VAR}` placeholders in their snapshot text.
// The form shows one input per variable (`extractTemplateVars`) and substitutes
// the values (`replaceTemplateVarsInText`). The values are also recorded in
// the provider's meta (`templateValues`) so re-editing a materialized snapshot
// can restore the placeholders (`restoreTemplatePlaceholders`) and pre-fill
// the inputs.

/** `${VAR}` placeholder pattern — letters, digits and underscores. */
const TEMPLATE_VAR_RE = /\$\{([A-Za-z_][A-Za-z0-9_]*)\}/g

/** Recursive walk over an arbitrary JSON structure: every string value passes
 *  through the transform. One traversal defines where placeholders live, so
 *  the extract / replace / restore helpers cannot drift apart. */
function walkStrings(
  value: unknown,
  transform: (s: string) => string,
): unknown {
  if (typeof value === "string") return transform(value)
  if (Array.isArray(value)) return value.map((v) => walkStrings(v, transform))
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([k, v]) => [
        k,
        walkStrings(v, transform),
      ]),
    )
  }
  return value
}

/** Collect the `${VAR}` placeholder names in any string value of a
 *  settingsConfig snapshot, deduped, in order of first appearance. The form
 *  shows one template-variable input per name; an empty list hides the
 *  section. */
export function extractTemplateVars(configText: string): string[] {
  const names: string[] = []
  walkStrings(parseSettingsConfig(configText), (s) => {
    for (const match of s.matchAll(TEMPLATE_VAR_RE)) {
      const name = match[1]!
      if (!names.includes(name)) names.push(name)
    }
    return s
  })
  return names
}

/** Replace every `${VAR}` placeholder in any string value of the snapshot with
 *  its value. A variable with no value — missing or empty — keeps its
 *  placeholder verbatim: the user simply did not fill it, and an empty string
 *  in an env key would silently corrupt the config. Callers check the result
 *  with `extractTemplateVars` before persisting. */
export function replaceTemplateVarsInText(
  configText: string,
  values: Record<string, string>,
): string {
  const config = parseSettingsConfig(configText)
  const next = walkStrings(config, (s) =>
    s.replace(TEMPLATE_VAR_RE, (match, name: string) => {
      const value = values[name]
      return value === undefined || value === "" ? match : value
    }),
  )
  return JSON.stringify(next, null, 2)
}

/** Restore the placeholders a previous save substituted, so re-editing a
 *  materialized snapshot behaves like the preset flow. Every occurrence of a
 *  recorded value reverts to its placeholder — a recorded value came from a
 *  placeholder by definition, and the values are distinctive strings (region
 *  codes, access keys), so this is safe in practice. Longer values are
 *  reverted first so a value that is a substring of another cannot be
 *  mangled. Strings without a recorded template are untouched. */
export function restoreTemplatePlaceholders(
  configText: string,
  values: Record<string, string>,
): string {
  const config = parseSettingsConfig(configText)
  const entries = Object.entries(values)
    .filter((entry) => entry[1] !== "")
    .sort((a, b) => b[1].length - a[1].length)
  const next = walkStrings(config, (s) => {
    let out = s
    for (const [name, value] of entries) {
      // split/join 代替 replaceAll：ES2020 目标库不支持 replaceAll，且值可能含正则元字符
      out = out.split(value).join(`\${${name}}`)
    }
    return out
  })
  return JSON.stringify(next, null, 2)
}

// ── Provider meta ──────────────────────────────────────────────────────────
//
// `meta` is app-side JSON that never reaches the live settings file. It
// records the template-variable values so the sheet can pre-fill the inputs
// and restore placeholders when re-editing.

/** App-side provider metadata. `templateValues` backs the preset template-
 *  variable inputs; `liveManaged` / `liveKey` back the additive-mode (opencode)
 *  "in live config" state — set by the add / remove / import commands. */
type ProviderMeta = {
  templateValues?: Record<string, string>
  /** 附加模式写盘标记：true = 已写进 opencode.json 的 `provider.<liveKey>`。
   *  add_provider_to_live / switch(opencode 分支) 置 true，
   *  remove_provider_from_live 置 false。单激活 app 永不写此字段。 */
  liveManaged?: boolean
  /** 附加模式下该供应商在 opencode.json 的 `provider.<key>` 键名。add / import
   *  时由后端派生（slugify 名称 / 沿用历史 / 回落 id）。 */
  liveKey?: string
}

/** Parse a provider's meta JSON text; garbage or empty → `{}` so a corrupt
 *  meta never throws the sheet open. A non-object `templateValues` is dropped
 *  to `{}`. */
export function parseMeta(metaText: string): ProviderMeta {
  if (!metaText) return {}
  try {
    const parsed: unknown = JSON.parse(metaText)
    if (typeof parsed !== "object" || parsed === null) return {}
    const meta = parsed as ProviderMeta
    if (
      meta.templateValues !== undefined &&
      (typeof meta.templateValues !== "object" ||
        meta.templateValues === null ||
        Array.isArray(meta.templateValues))
    ) {
      return { ...meta, templateValues: {} }
    }
    return meta
  } catch {
    return {}
  }
}

/** The template-variable values recorded in the meta (string entries only —
 *  a hand-edited meta could hold garbage). */
export function metaTemplateValues(metaText: string): Record<string, string> {
  const values = parseMeta(metaText).templateValues
  if (!values) return {}
  return Object.fromEntries(
    Object.entries(values).filter(
      (entry): entry is [string, string] => typeof entry[1] === "string",
    ),
  )
}

/** Record the current template-variable values in the meta text, replacing
 *  the previous record and keeping unknown meta keys. Empty values are
 *  dropped and an empty map removes the key, so a provider that no longer
 *  uses placeholders stays clean. */
export function withMetaTemplateValues(
  metaText: string,
  values: Record<string, string>,
): string {
  const meta = parseMeta(metaText)
  const filled = Object.fromEntries(
    Object.entries(values).filter((entry) => entry[1] !== ""),
  )
  if (Object.keys(filled).length === 0) delete meta.templateValues
  else meta.templateValues = filled
  return JSON.stringify(meta, null, 2)
}

/** 附加模式（opencode）：该供应商是否已写进 live 配置（opencode.json 的
 *  `provider.<liveKey>`）。严格 true 判定——meta 里非布尔的垃圾值不算。 */
export function providerLiveManaged(provider: Provider): boolean {
  return parseMeta(provider.meta).liveManaged === true
}

/** 附加模式（opencode）：该供应商在 live 配置里的键名（`provider.<key>`）。
 *  空 = 尚未写盘或 meta 缺失；非字符串的垃圾值归一为空。 */
export function providerLiveKey(provider: Provider): string {
  const key = parseMeta(provider.meta).liveKey
  return typeof key === "string" ? key : ""
}
