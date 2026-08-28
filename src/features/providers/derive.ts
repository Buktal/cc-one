// 供应商表单派生逻辑的聚合层（架构扫描候选⑤拆分后）：各应用的 settingsConfig
// codec 在 codecs/（claude/codex/gemini/grok/opencode.ts，各自 parse/with 家族）、
// 片段域在 snippet.ts、必填检查在 missing.ts、模板变量在 template-vars.ts
// （后者独立成文件——missing 也要用它，留在本文件会成 missing ↔ derive 循环
// import）。本文件保留跨应用的「表单聚合」逻辑——app 侧 meta 记录、列表过滤
// 与新建/预设草稿——并 `export *` 重导出各域符号，保持既有调用方的
// `from "@/features/providers/derive"` import 不变。改单个 app 的 codec 不再
// 需要打开这个聚合文件。

import { APP_PROFILES } from "@/features/providers/app-profiles"
import type { ProviderPreset } from "@/features/providers/presets"
import { parseJsonObjectLenient } from "@/lib/json"
import type { App, Provider } from "@/types/generated/bindings"

export * from "./codecs/claude"
export * from "./codecs/codex"
export * from "./codecs/gemini"
export * from "./codecs/grok"
export * from "./codecs/opencode"
export * from "./missing"
export * from "./snippet"
export * from "./template-vars"

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
 *  config — the form starts fresh). `app` defaults to claude (the original
 *  pool); the blank settingsConfig shape is an app-profile fact (claude =
 *  `{"env": {}}` container, the rest `"{}"`). `id` is empty so
 *  `save_provider_cmd` allocates a fresh one. */
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
    settingsConfig: APP_PROFILES[app].newDraftText,
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
    // notes 是 Provider 的入库契约（Rust 模型 + DB 列 + 同步工件都有它），
    // 前端类型必须保留；预设源不再携带 notes（死数据，全仓无 UI 读端），
    // 草稿一律落空串。
    notes: "",
    settingsConfig: preset.settingsConfig,
    meta: "{}",
    updatedAt: "",
  }
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
  const parsed = parseJsonObjectLenient(metaText)
  if (!parsed) return {}
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
