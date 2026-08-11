// Pure derivations for the providers list & form: reading the basic form
// fields (endpoint / API key) and the five-role model mapping (Sonnet / Opus /
// Haiku / Fable / Subagent models, display names, 1M marker) out of a
// provider's `settingsConfig` snapshot, and rebuilding that snapshot from the
// form while preserving every other field. Also the auth-field toggle (moving
// the key between the two spellings), the `${VAR}` template-variable
// substitution for preset snapshots, and the app-side `meta` record that makes
// re-editing work. Extracted from the view/hook so each rule is testable in
// isolation — the single authority for how the form maps onto the settings.json
// text.

import type { ProviderPreset } from "@/features/providers/presets"
import type { App, Provider } from "@/types/generated/bindings"

// The env keys the basic form knows about. The auth key has two spellings
// Claude Code accepts — AUTH_TOKEN (the default the form writes) and API_KEY
// (the legacy spelling some providers require; the form reads either, so a
// provider configured with API_KEY edits cleanly).
const ENV_BASE_URL = "ANTHROPIC_BASE_URL"
const ENV_AUTH_TOKEN = "ANTHROPIC_AUTH_TOKEN"
const ENV_API_KEY = "ANTHROPIC_API_KEY"
const ENV_MODEL = "ANTHROPIC_MODEL"

/** The legacy small/fast model key — Haiku's backfill source. Model writes
 *  delete it, so a snapshot never keeps both the new role keys and the old
 *  spelling that preceded them. */
const ENV_SMALL_FAST_MODEL = "ANTHROPIC_SMALL_FAST_MODEL"

/** The `[1M]` suffix declaring the 1M-context capability — a marker Claude
 *  Code reads natively off the model name in env (e.g. "claude-opus-5[1M]"),
 *  appended when the form's 1M box is checked. */
const ONE_M_MARKER = "[1M]"

type SettingsConfig = { env?: Record<string, string> }

/** Parse a provider's settingsConfig JSON text; garbage or empty → `{}` so a
 *  corrupt snapshot never throws the form open. A non-object `env` (a string,
 *  an array — anything a hand-edited snapshot could hold) is dropped to `{}`
 *  so the write-back never spreads e.g. a string into character-index keys. */
export function parseSettingsConfig(config: string): SettingsConfig {
  if (!config) return {}
  try {
    const parsed: unknown = JSON.parse(config)
    if (typeof parsed !== "object" || parsed === null) return {}
    const cfg = parsed as SettingsConfig
    if (
      cfg.env !== undefined &&
      (typeof cfg.env !== "object" ||
        cfg.env === null ||
        Array.isArray(cfg.env))
    ) {
      return { ...cfg, env: {} }
    }
    return cfg
  } catch {
    return {}
  }
}

function envValue(configText: string, key: string): string {
  return parseSettingsConfig(configText).env?.[key] ?? ""
}

/** The provider's base URL (endpoint), from `env.ANTHROPIC_BASE_URL`. */
export function providerEndpoint(provider: Provider): string {
  return envValue(provider.settingsConfig, ENV_BASE_URL)
}

/** The API key — reads AUTH_TOKEN first, then API_KEY (the form writes the
 *  former by default; the latter is the legacy spelling some providers use). */
export function providerApiKey(provider: Provider): string {
  return (
    envValue(provider.settingsConfig, ENV_AUTH_TOKEN) ||
    envValue(provider.settingsConfig, ENV_API_KEY)
  )
}

/** Text-level twin of `providerEndpoint` — reads the endpoint straight from a
 *  settingsConfig JSON text (the JSON editor's working value). */
export function configEndpoint(configText: string): string {
  return envValue(configText, ENV_BASE_URL)
}

/** Text-level twin of `providerApiKey` — reads the API key straight from a
 *  settingsConfig JSON text, AUTH_TOKEN first then the legacy API_KEY. */
export function configApiKey(configText: string): string {
  return (
    envValue(configText, ENV_AUTH_TOKEN) || envValue(configText, ENV_API_KEY)
  )
}

/** The primary model, from `env.ANTHROPIC_MODEL`. The basic form does not own
 *  it — it is the fallback the role mapping reads when a role key is missing. */
export function providerModel(provider: Provider): string {
  return envValue(provider.settingsConfig, ENV_MODEL)
}

/** 切换前必填项检查：缺失的部分（端点、API key、未物化的模板变量）。
 *  「缺失」与表单收集语义一致——端点取 `ANTHROPIC_BASE_URL`、key 取
 *  AUTH_TOKEN 优先后 API_KEY。官方 / 云厂商预设供应商不要求端点/key
 *  （Claude Official 走默认端点，Bedrock 用模板变量认证，见 presets）；
 *  它们和自定义供应商一样，仍受模板变量残留检查约束。空数组 = 没有缺失。 */
export function providerMissingRequired(provider: Provider): string[] {
  const missing: string[] = []
  if (
    provider.category !== "official" &&
    provider.category !== "cloud_provider" &&
    !providerEndpoint(provider).trim()
  ) {
    missing.push("endpoint")
  }
  if (
    provider.category !== "official" &&
    provider.category !== "cloud_provider" &&
    !providerApiKey(provider).trim()
  ) {
    missing.push("apiKey")
  }
  if (extractTemplateVars(provider.settingsConfig).length > 0) {
    missing.push("templateVars")
  }
  return missing
}

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

/**
 * Merge the basic form fields (endpoint / API key) into a settingsConfig JSON
 * text, keeping every field the form does not own (extra env keys, non-env
 * settings) untouched — the text-level twin of `withBasicFields` that the form
 * sheet uses to keep the JSON editor in sync while typing. Callers must only
 * pass config that parses to an object (`parseJsonObject`), else a garbage
 * snapshot would be replaced by a bare `{"env": …}` and the in-progress edit
 * lost. A non-empty key is written under the selected auth field
 * (`fields.authField`, AUTH_TOKEN by default) and the other spelling dropped;
 * an empty endpoint / key removes the stale env entry.
 */
export function withBasicFieldsInText(
  configText: string,
  fields: { endpoint: string; apiKey: string; authField?: AuthField },
): string {
  const config = parseSettingsConfig(configText)
  const env = { ...(config.env ?? {}) }
  if (fields.endpoint) env[ENV_BASE_URL] = fields.endpoint
  else delete env[ENV_BASE_URL]
  if (fields.apiKey) {
    const target = authFieldKey(fields.authField ?? "auth_token")
    env[target] = fields.apiKey
    delete env[target === ENV_AUTH_TOKEN ? ENV_API_KEY : ENV_AUTH_TOKEN]
  } else {
    delete env[ENV_AUTH_TOKEN]
    delete env[ENV_API_KEY]
  }
  return JSON.stringify({ ...config, env }, null, 2)
}

/**
 * Rebuild a provider's settingsConfig from the basic form fields, keeping
 * every field the form does not own (extra env keys, non-env settings)
 * untouched — an endpoint / key edit must never drop the rest of a snapshot.
 * A non-empty key is written under the selected auth field
 * (`fields.authField`, AUTH_TOKEN by default) and the other spelling dropped,
 * so a provider that only carried API_KEY migrates to one credential instead
 * of leaving both in the snapshot — and a provider toggled to API_KEY stays
 * on that spelling. An empty endpoint / key removes the stale env entry, so
 * clearing a field in the form clears it in the snapshot too (both key
 * spellings on clear).
 */
export function withBasicFields(
  provider: Provider,
  fields: { endpoint: string; apiKey: string; authField?: AuthField },
): Provider {
  return {
    ...provider,
    settingsConfig: withBasicFieldsInText(provider.settingsConfig, fields),
  }
}

// ------------------------------------------------------- model roles + 1M --

/** The five model roles Claude Code routes to. Each role carries its own
 *  request model, an optional display name for the model picker, and — except
 *  Haiku — a 1M-capability checkbox. */
export type ModelRoleId = "sonnet" | "opus" | "haiku" | "fable" | "subagent"

/** A role's env mapping: where its model and display name live, what to fall
 *  back to when the model key is missing, and whether it may declare the 1M
 *  context marker (Haiku cannot — it is stripped on write). */
export interface ModelRole {
  id: ModelRoleId
  modelKey: string
  nameKey: string
  /** Env keys tried in order when `modelKey` is missing — mirrors the runtime
   *  mapping chain so the form never shows a hole a configured provider fills. */
  backfillKeys: string[]
  supportsOneM: boolean
}

/** The five roles, in form-display order — single source of truth for the env
 *  key mapping: the form iterates this table and the helpers look roles up in
 *  it. Backfill chains: Haiku falls back to the legacy small-fast key, then
 *  the primary model; Fable through Opus's key, Subagent through Sonnet's, the
 *  rest to the primary model. */
export const MODEL_ROLES: ModelRole[] = [
  {
    id: "sonnet",
    modelKey: "ANTHROPIC_DEFAULT_SONNET_MODEL",
    nameKey: "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
    backfillKeys: [ENV_MODEL],
    supportsOneM: true,
  },
  {
    id: "opus",
    modelKey: "ANTHROPIC_DEFAULT_OPUS_MODEL",
    nameKey: "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
    backfillKeys: [ENV_MODEL],
    supportsOneM: true,
  },
  {
    id: "haiku",
    modelKey: "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    nameKey: "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
    backfillKeys: [ENV_SMALL_FAST_MODEL, ENV_MODEL],
    supportsOneM: false,
  },
  {
    id: "fable",
    modelKey: "ANTHROPIC_DEFAULT_FABLE_MODEL",
    nameKey: "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    backfillKeys: ["ANTHROPIC_DEFAULT_OPUS_MODEL", ENV_MODEL],
    supportsOneM: true,
  },
  {
    id: "subagent",
    modelKey: "ANTHROPIC_DEFAULT_SUBAGENT_MODEL",
    nameKey: "ANTHROPIC_DEFAULT_SUBAGENT_MODEL_NAME",
    backfillKeys: ["ANTHROPIC_DEFAULT_SONNET_MODEL", ENV_MODEL],
    supportsOneM: true,
  },
]

function modelRole(role: ModelRoleId): ModelRole {
  const def = MODEL_ROLES.find((r) => r.id === role)
  if (!def) throw new Error(`Unknown model role: ${role}`)
  return def
}

/** Whether a model name carries the 1M marker. Read case-insensitively —
 *  proxies forward the marker lowercase upstream, Claude Code accepts both
 *  spellings, so the form strips either. */
export function hasOneM(model: string): boolean {
  return model.trimEnd().toLowerCase().endsWith("[1m]")
}

/** Strip a trailing 1M marker, leaving the bare model name. No marker → the
 *  input is returned unchanged; only the marker at the very end (with any
 *  trailing whitespace) is removed. */
export function stripOneM(model: string): string {
  if (!hasOneM(model)) return model
  return model.trimEnd().slice(0, -ONE_M_MARKER.length).trimEnd()
}

/** Apply (oneM) or remove the 1M marker — idempotent: an already-marked model
 *  is stripped first, so toggling never stacks markers. An empty model stays
 *  empty. */
export function setModelOneM(model: string, oneM: boolean): string {
  const base = stripOneM(model).trim()
  if (!base) return ""
  return oneM ? `${base}${ONE_M_MARKER}` : base
}

/** A role's effective model — its own env key first, then the backfill chain,
 *  then "". The raw env value is returned, `[1M]` marker included (the marker
 *  is a property of the model value, not of the read). */
export function configRoleModel(configText: string, role: ModelRoleId): string {
  const def = modelRole(role)
  const env = parseSettingsConfig(configText).env ?? {}
  const direct = env[def.modelKey]
  if (direct) return direct
  for (const key of def.backfillKeys) {
    const backfill = env[key]
    if (backfill) return backfill
  }
  return ""
}

/** A role's display name — the `_NAME` key, or the marker-free model name
 *  (the picker shows bare names, never the `[1M]` suffix). */
export function configRoleName(configText: string, role: ModelRoleId): string {
  const def = modelRole(role)
  const env = parseSettingsConfig(configText).env ?? {}
  return env[def.nameKey] || stripOneM(configRoleModel(configText, role))
}

/** Whether the role declares the 1M capability — its effective model carries
 *  the marker. Roles that do not support 1M always read false, even if a
 *  hand-edited snapshot carries a stray marker. */
export function configRoleHasOneM(
  configText: string,
  role: ModelRoleId,
): boolean {
  const def = modelRole(role)
  return def.supportsOneM && hasOneM(configRoleModel(configText, role))
}

/** The three fields the form edits per role, read together from the snapshot. */
export interface RoleFields {
  model: string
  name: string
  oneM: boolean
}

export function configRoleFields(
  configText: string,
  role: ModelRoleId,
): RoleFields {
  return {
    model: configRoleModel(configText, role),
    name: configRoleName(configText, role),
    oneM: configRoleHasOneM(configText, role),
  }
}

/** Rewrite a settingsConfig text's env via `write`, preserving every other
 *  field — the shared engine behind the role writes. */
function withEnvInText(
  configText: string,
  write: (env: Record<string, string>) => void,
): string {
  const config = parseSettingsConfig(configText)
  const env = { ...(config.env ?? {}) }
  write(env)
  return JSON.stringify({ ...config, env }, null, 2)
}

/**
 * Write a role's model into the settingsConfig text, syncing the display name
 * by the rule: when the role has no display name yet, or its display name
 * equals the old model name (marker stripped), it follows the new model —
 * a hand-typed model update keeps the picker label in step without clobbering
 * a custom name. Haiku (no 1M support) is stripped of any marker on write;
 * the other roles keep one typed or toggled in. Every write deletes the legacy
 * small-fast key — the role keys supersede it, and it must not linger
 * alongside them. An empty model clears the key (and a synced display name).
 */
export function withRoleModelInText(
  configText: string,
  role: ModelRoleId,
  model: string,
): string {
  const def = modelRole(role)
  const oldModelBase = stripOneM(configRoleModel(configText, role)).trim()
  const written = def.supportsOneM ? model.trim() : stripOneM(model)
  return withEnvInText(configText, (env) => {
    if (written) env[def.modelKey] = written
    else delete env[def.modelKey]
    delete env[ENV_SMALL_FAST_MODEL]
    const name = (env[def.nameKey] ?? "").trim()
    if (!name || name === oldModelBase) {
      const nextName = stripOneM(written).trim()
      if (nextName) env[def.nameKey] = nextName
      else delete env[def.nameKey]
    }
  })
}

/** Write a role's display name. Empty clears the key so the read-time default
 *  (the marker-free model name) shows again. */
export function withRoleNameInText(
  configText: string,
  role: ModelRoleId,
  name: string,
): string {
  const def = modelRole(role)
  return withEnvInText(configText, (env) => {
    const trimmed = name.trim()
    if (trimmed) env[def.nameKey] = trimmed
    else delete env[def.nameKey]
  })
}

/** Toggle the 1M marker on a role's model. Roles that do not support 1M are
 *  left untouched. The marker goes through the same write path as a typed
 *  model, so the display-name sync rule applies. */
export function withRoleOneMInText(
  configText: string,
  role: ModelRoleId,
  oneM: boolean,
): string {
  const def = modelRole(role)
  if (!def.supportsOneM) return configText
  return withRoleModelInText(
    configText,
    role,
    setModelOneM(configRoleModel(configText, role), oneM),
  )
}

/** Write one model to every role — the shared engine behind the one-click
 *  apply and the fetched-model picker refill. Same per-role semantics as a
 *  typed model (display-name sync, Haiku marker strip, small-fast key
 *  deletion), so the two entry points can never drift apart. */
export function withAllRolesInText(configText: string, model: string): string {
  let next = configText
  for (const def of MODEL_ROLES) {
    next = withRoleModelInText(next, def.id, model)
  }
  return next
}

/**
 * One-click apply: take the first filled model — the primary model, then the
 * roles in display order — and write it to every role via
 * `withAllRolesInText`, syncing display names (marker-free) and stripping the
 * marker for Haiku. A picked model that carries `[1M]` propagates the marker
 * to the roles that support it. Returns null when no model is filled anywhere
 * (callers disable the button).
 */
export function withAllRolesFromFirstInText(configText: string): string | null {
  const env = parseSettingsConfig(configText).env ?? {}
  const candidates = [
    env[ENV_MODEL],
    ...MODEL_ROLES.map((r) => env[r.modelKey]),
  ]
  const picked = candidates.find((m) => m?.trim())
  if (!picked) return null
  return withAllRolesInText(configText, picked)
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

/** Build the "new provider" draft from a built-in preset: category goes to
 *  `custom` (a preset is the starting point, customization is the end — the
 *  saved row is a user provider, not a preset), `id` stays empty so
 *  `save_provider_cmd` allocates a fresh one. The preset's settingsConfig
 *  snapshot is copied verbatim (its `${VAR}` placeholders stay until the
 *  template-variable step); the preset constant itself is never mutated.
 *  `app` defaults to claude — the preset arrays are app-scoped (see
 *  `presetsForApp`), so the caller knows which pool the preset came from. */
export function providerFromPreset(
  preset: ProviderPreset,
  app: App = "claude",
): Provider {
  return {
    id: "",
    name: preset.name,
    websiteUrl: preset.websiteUrl,
    category: "custom",
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

// ── Auth field toggle ──────────────────────────────────────────────────────
//
// The form can write the API key under either of the two env spellings Claude
// Code accepts. `switchAuthField` moves the value between them so toggling
// never loses or duplicates the credential; `configAuthField` derives the
// current field from the snapshot (the JSON editor stays the source of
// truth).

/** The two env keys the auth-field toggle can write to. AUTH_TOKEN is the
 *  default (what Claude Code documents); API_KEY is the legacy spelling some
 *  providers require. */
export type AuthField = "auth_token" | "api_key"

/** The env key a field value maps to. */
export function authFieldKey(field: AuthField): string {
  return field === "auth_token" ? ENV_AUTH_TOKEN : ENV_API_KEY
}

/** Which auth field the snapshot currently uses. API_KEY only when that is
 *  the sole spelling present — with both, or neither, the default AUTH_TOKEN
 *  wins, mirroring the read preference of `providerApiKey`. */
export function configAuthField(configText: string): AuthField {
  const env = parseSettingsConfig(configText).env ?? {}
  return env[ENV_AUTH_TOKEN] === undefined && env[ENV_API_KEY] !== undefined
    ? "api_key"
    : "auth_token"
}

/** Move the API key value from one auth field to the other and delete the old
 *  key, so the toggle never loses or duplicates the credential. The rest of
 *  the snapshot is untouched; a missing value just removes the old key. A
 *  no-op when `from === to`. */
export function switchAuthField(
  configText: string,
  from: AuthField,
  to: AuthField,
): string {
  if (from === to) return configText
  const config = parseSettingsConfig(configText)
  const env = { ...(config.env ?? {}) }
  const value = env[authFieldKey(from)]
  delete env[authFieldKey(from)]
  if (value !== undefined) env[authFieldKey(to)] = value
  return JSON.stringify({ ...config, env }, null, 2)
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

/** App-side provider metadata — the only consumer today is the template
 *  variable record. */
type ProviderMeta = { templateValues?: Record<string, string> }

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

// ---- 通用配置片段（snippet）----
//
// `snippetMissingKeys` 是「片段子集判定」：对比片段与某份 settingsConfig，
// 报告片段里存在、而配置里缺失的受控字段——这些是切换写盘时片段实际会补上
// 的部分。写盘合并的权威在 Rust（provider::snippet 只认受控字段），这里只
// 为 UI 提示（片段卡片显示当前激活供应商会从片段得到什么）维护同一份受控
// 字段清单，必须与后端 `provider::live::CONTROLLED_FIELDS` 保持同步。

/** 写盘路径承认的受控字段（镜像后端常量；仅供 UI 提示使用，写盘权威在后端）。 */
const CONTROLLED_FIELDS = [
  "env",
  "includeCoAuthoredBy",
  "attribution",
  "effortLevel",
  "enabledPlugins",
  "skipWebFetchPreflight",
] as const

/** 片段子集判定的严格解析：空串 → `{}`；非空但非法 JSON 或非对象 → `null`
 *  ——解析不了的输入没法可靠判定缺失，调用方对 `null` 一律报 `[]`，不误导
 *  （与 `parseSettingsConfig` 的宽容契约不同：它把垃圾吞成 `{}` 以让表单
 *  不崩，而这里空与垃圾必须分得清，否则垃圾配置会误报「片段将补 X」）。 */
function parseSnippetInput(text: string): Record<string, unknown> | null {
  if (!text) return {}
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return null
    }
    return parsed as Record<string, unknown>
  } catch {
    return null
  }
}

/** env 非对象（手写垃圾）按空对象处理——与后端合并语义一致：非对象 env
 *  被跳过、不参与键级判定。 */
function envRecordOf(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
}

/** 片段对 settingsConfig 的补充（子集判定）：片段里出现、而配置里缺失的
 *  受控字段键——这些是切换写盘时片段会补上的部分。`env` 按键级判定：片段
 *  env 有任一键缺失即报告 `env`。非受控键不算（写盘合并时被忽略，报了是
 *  误导）。配置/片段为空 → `[]`；非空但解析不了或不是对象 → `[]`。 */
export function snippetMissingKeys(
  configText: string,
  snippetText: string,
): string[] {
  const config = parseSnippetInput(configText)
  const snippet = parseSnippetInput(snippetText)
  if (config === null || snippet === null) return []
  const missing: string[] = []
  for (const key of CONTROLLED_FIELDS) {
    if (key === "env") {
      const configEnv = envRecordOf(config.env)
      const snippetEnv = envRecordOf(snippet.env)
      if (Object.keys(snippetEnv).some((k) => !(k in configEnv))) {
        missing.push("env")
      }
    } else if (key in snippet && !(key in config)) {
      missing.push(key)
    }
  }
  return missing
}

// ---- Codex 应用：auth + TOML 受控合并的文本级读写 ----
//
// Codex settingsConfig 形状：`{"auth": {"OPENAI_API_KEY"?: ...}, "config":
// "<TOML 文本>"}`。`auth.OPENAI_API_KEY` 为空 / 缺失 = 登录态版（不写
// auth.json）；`config` 是 TOML 文本，写盘时按受控键整块合并进
// ~/.codex/config.toml（用户手动的非受控字段原样保留）。纯函数：写盘语义
// 的镜像，但更宽容（坏输入不抛、归一为空），用于表单读写。

type CodexConfig = { auth: Record<string, string>; config: string }

/** 解析 Codex settingsConfig 文本为 `{auth, config}`。宽容：空 / 垃圾 / 非对象
 *  → `{auth:{}, config:""}`；非对象 auth 当 `{}`、非字符串 config 当 `""`
 *  处理——表单遇到手改坏的快照也不崩，写回时按归一结果继续。 */
export function parseCodexConfig(text: string): CodexConfig {
  if (!text) return { auth: {}, config: "" }
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { auth: {}, config: "" }
    }
    const obj = parsed as { auth?: unknown; config?: unknown }
    const auth =
      obj.auth !== null &&
      typeof obj.auth === "object" &&
      !Array.isArray(obj.auth)
        ? Object.fromEntries(
            Object.entries(obj.auth as Record<string, unknown>).filter(
              (entry): entry is [string, string] =>
                typeof entry[1] === "string",
            ),
          )
        : {}
    const config = typeof obj.config === "string" ? obj.config : ""
    return { auth, config }
  } catch {
    return { auth: {}, config: "" }
  }
}

/** 读 Codex settingsConfig 的 `auth.OPENAI_API_KEY`（API Key 版的凭据）。 */
export function codexApiKey(text: string): string {
  return parseCodexConfig(text).auth.OPENAI_API_KEY ?? ""
}

/** 读 Codex settingsConfig 的 `config`（config.toml 的 TOML 文本）。 */
export function codexConfigToml(text: string): string {
  return parseCodexConfig(text).config
}

/** 把 API Key 写入 Codex settingsConfig 的 `auth.OPENAI_API_KEY`：非空 key 写入、
 *  空 key 删除该键（回归登录态版）。保留 auth 其余键与 config 字段不动。 */
export function withCodexApiKey(text: string, key: string): string {
  const { auth, config } = parseCodexConfig(text)
  const next = { ...auth }
  if (key) next.OPENAI_API_KEY = key
  else delete next.OPENAI_API_KEY
  return JSON.stringify({ auth: next, config }, null, 2)
}

/** 把 TOML 文本写入 Codex settingsConfig 的 `config` 字段。保留 auth 与其余
 *  字段不动（受控合并的目标值就是这里整段 TOML）。 */
export function withCodexConfigToml(text: string, toml: string): string {
  const { auth } = parseCodexConfig(text)
  return JSON.stringify({ auth, config: toml }, null, 2)
}

// ---- Gemini 应用：env 整块写 .env，config 合并进 settings.json ----
//
// Gemini settingsConfig 形状：`{"env": {...}, "config"?: {...}}`。`env` 整块
// 替换 ~/.gemini/.env；`config` 合并进 ~/.gemini/settings.json 的受控字段。
// env 含 `GEMINI_API_KEY` → API Key 版（selectedType="gemini-api-key"），
// 否则 → 登录态版（selectedType="oauth"，保留 Google 登录）。`config` 在
// 这里作为原始 JSON 文本返回，便于编辑器编辑。

type GeminiConfig = { env: Record<string, string>; config: string }

/** 解析 Gemini settingsConfig 文本为 `{env, config}`。config 字段以原始 JSON
 *  文本返回（`config ? JSON.stringify(config) : ""`），便于编辑器编辑与回写。
 *  宽容：空 / 垃圾 / 非对象 → `{env:{}, config:""}`；非对象 env 当 `{}`、非
 *  对象 config 当 `""` 处理——表单遇到手改坏的快照也不崩。 */
export function parseGeminiConfig(text: string): GeminiConfig {
  if (!text) return { env: {}, config: "" }
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { env: {}, config: "" }
    }
    const obj = parsed as { env?: unknown; config?: unknown }
    const env =
      obj.env !== null && typeof obj.env === "object" && !Array.isArray(obj.env)
        ? Object.fromEntries(
            Object.entries(obj.env as Record<string, unknown>).filter(
              (entry): entry is [string, string] =>
                typeof entry[1] === "string",
            ),
          )
        : {}
    const config =
      obj.config !== null &&
      typeof obj.config === "object" &&
      !Array.isArray(obj.config)
        ? JSON.stringify(obj.config)
        : ""
    return { env, config }
  } catch {
    return { env: {}, config: "" }
  }
}

/** 读 Gemini settingsConfig 的 `env.GEMINI_API_KEY`（API Key 版的判据 + 凭据）。 */
export function geminiApiKey(text: string): string {
  return parseGeminiConfig(text).env.GEMINI_API_KEY ?? ""
}

/** 读 Gemini settingsConfig 的 `env.GEMINI_MODEL`（主模型名）。 */
export function geminiModel(text: string): string {
  return parseGeminiConfig(text).env.GEMINI_MODEL ?? ""
}

/** 读 Gemini settingsConfig 的 `env.GOOGLE_GEMINI_BASE_URL`（端点）。 */
export function geminiBaseUrl(text: string): string {
  return parseGeminiConfig(text).env.GOOGLE_GEMINI_BASE_URL ?? ""
}

/** 把 patch 的键合并进 Gemini settingsConfig 的 env：值为空串则删除该键
 *  （`GEMINI_API_KEY` 删除即回归登录态版），非空则写入 / 覆盖。保留 config
 *  字段与其余 env 键不动。 */
export function withGeminiEnv(
  text: string,
  patch: Record<string, string>,
): string {
  const { env, config } = parseGeminiConfig(text)
  const next = { ...env }
  for (const [key, value] of Object.entries(patch)) {
    if (value) next[key] = value
    else delete next[key]
  }
  // 配置回写以 parseGeminiConfig 解出的 JSON 文本为准——保留原始结构而非
  // 重新序列化，避免合法 config 被空对象覆盖。
  const configObj: Record<string, unknown> | undefined = config
    ? (JSON.parse(config) as Record<string, unknown>)
    : undefined
  return JSON.stringify(
    configObj ? { env: next, config: configObj } : { env: next },
    null,
    2,
  )
}

/** 把 JSON 文本写入 Gemini settingsConfig 的 `config` 字段。空串 → 删除
 *  config 键；非空 → 尝试 JSON.parse，非法 → 原样不动返回原 text（编辑器
 *  正在敲半截 JSON 时不会被吞）。保留 env 字段不动。 */
export function withGeminiConfigJson(text: string, configJson: string): string {
  const { env } = parseGeminiConfig(text)
  if (!configJson) {
    return JSON.stringify({ env }, null, 2)
  }
  try {
    const parsed = JSON.parse(configJson) as unknown
    if (
      parsed === null ||
      typeof parsed !== "object" ||
      Array.isArray(parsed)
    ) {
      return text
    }
    return JSON.stringify(
      { env, config: parsed as Record<string, unknown> },
      null,
      2,
    )
  } catch {
    return text
  }
}

// ---- Grok 应用：命名 profile TOML 的文本级读写 ----
//
// Grok settingsConfig 形状：`{"config": "<TOML 文本>"}`。config 即
// ~/.grok/config.toml 的受控片段——`[model.cc-one]` profile 块（写盘时整块替换
// + 设 models.default）。空 config = 登录态版（官方，Grok CLI 回落自带 xAI
// OAuth）。表单是纯 TOML 编辑器——api_key / model / base_url 等字段都在 TOML
// 内，前端无 TOML 解析器，故不拆结构化字段（与 codex 的 JSON-auth 抽离不同）。

type GrokConfig = { config: string }

/** 解析 Grok settingsConfig 文本为 `{config}`。宽容：空 / 垃圾 / 非对象 →
 *  `{config:""}`；非字符串 config 当 "" 处理——表单遇到手改坏的快照也不崩，
 *  写回时按归一结果继续。 */
export function parseGrokConfig(text: string): GrokConfig {
  if (!text) return { config: "" }
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { config: "" }
    }
    const obj = parsed as { config?: unknown }
    return { config: typeof obj.config === "string" ? obj.config : "" }
  } catch {
    return { config: "" }
  }
}

/** 读 Grok settingsConfig 的 `config`（config.toml 的 TOML 文本，即
 *  `[model.cc-one]` profile 块）。 */
export function grokConfigToml(text: string): string {
  return parseGrokConfig(text).config
}

/** 把 TOML 文本写入 Grok settingsConfig 的 `config` 字段（受控合并的目标值
 *  就是这里整段 TOML）。与 codex 的 withCodexConfigToml 同一「先 parse 再
 *  stringify」形态。 */
export function withGrokConfigToml(text: string, toml: string): string {
  const parsed = parseGrokConfig(text)
  return JSON.stringify({ ...parsed, config: toml }, null, 2)
}
