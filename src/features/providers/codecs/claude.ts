// Claude 应用的 settingsConfig codec：读整份 settings.json 快照里的基础表单
// 字段（端点 / API key / 主模型）与五角色模型映射，并把表单改写的字段写回快照
// 文本、保留其余字段。Claude 的 settingsConfig 形状 = `{"env": {...}}`，本 codec
// 同时承载「JSON 快照文本」的共享解析基础（`parseSettingsConfig`），供模板变量 /
// meta 等聚合逻辑复用。auth 键有两种拼写（AUTH_TOKEN 默认 / API_KEY 旧拼写），
// 切换拼写由 `switchAuthField` 一族承担。写盘合并的权威在 Rust（live.rs）；这里
// 是表单侧的行为镜像，纯函数、可测。

import { parseJsonObjectLenient } from "@/lib/json"
import type { Provider } from "@/types/generated/bindings"

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
  const obj = parseJsonObjectLenient(config)
  if (!obj) return {}
  const cfg = obj as SettingsConfig
  if (
    cfg.env !== undefined &&
    (typeof cfg.env !== "object" || cfg.env === null || Array.isArray(cfg.env))
  ) {
    return { ...cfg, env: {} }
  }
  return cfg
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
 *  apply, the fetched-model picker refill and the auto-sync toggle. The
 *  propagated value is the bare model name: each role's OWN 1M checkbox state
 *  decides the marker (a role that currently carries `[1M]` keeps it, one
 *  that doesn't stays bare) — unify the model, never the toggle. Otherwise a
 *  marker-free primary model (the first candidate) would wipe every role's 1M.
 *  Haiku never takes the marker. Same per-role semantics as a typed model
 *  (display-name sync, small-fast key deletion), so the entry points can't
 *  drift apart. */
export function withAllRolesInText(configText: string, model: string): string {
  const bare = stripOneM(model).trim()
  let next = configText
  for (const def of MODEL_ROLES) {
    if (!bare) {
      next = withRoleModelInText(next, def.id, "")
      continue
    }
    const roleHasOneM =
      def.supportsOneM && hasOneM(configRoleModel(configText, def.id))
    next = withRoleModelInText(next, def.id, roleHasOneM ? `${bare}[1M]` : bare)
  }
  return next
}

/**
 * One-click apply: take the first filled model — the primary model, then the
 * roles in display order — and write it to every role via
 * `withAllRolesInText`, syncing display names (marker-free). The marker
 * follows each role's own 1M checkbox state, never the picked model's.
 * Returns null when no model is filled anywhere (callers disable the button).
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
