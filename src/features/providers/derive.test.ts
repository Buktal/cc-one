import { describe, expect, it } from "vitest"
import { CODEX_PROVIDER_PRESETS } from "@/features/providers/codex-presets"
import {
  authFieldKey,
  codexApiKey,
  codexConfigToml,
  configApiKey,
  configAuthField,
  configEndpoint,
  configRoleFields,
  configRoleHasOneM,
  configRoleModel,
  configRoleName,
  emptyProvider,
  extractTemplateVars,
  filterProviders,
  geminiApiKey,
  geminiBaseUrl,
  geminiModel,
  grokConfigToml,
  hasOneM,
  metaTemplateValues,
  openCodeApiKey,
  openCodeBaseUrl,
  openCodeHeaders,
  openCodeModels,
  openCodeNpm,
  parseCodexConfig,
  parseGeminiConfig,
  parseGrokConfig,
  parseOpenCodeConfig,
  providerApiKey,
  providerEndpoint,
  providerFromPreset,
  providerLiveKey,
  providerLiveManaged,
  providerMissingRequired,
  providerModel,
  replaceTemplateVarsInText,
  restoreTemplatePlaceholders,
  setModelOneM,
  snippetMissingKeys,
  stripOneM,
  switchAuthField,
  withAllRolesFromFirstInText,
  withAllRolesInText,
  withBasicFields,
  withBasicFieldsInText,
  withCodexApiKey,
  withCodexConfigToml,
  withGeminiConfigJson,
  withGeminiEnv,
  withGrokConfigToml,
  withMetaTemplateValues,
  withOpenCodeApiKey,
  withOpenCodeBaseUrl,
  withOpenCodeHeaders,
  withOpenCodeModels,
  withOpenCodeNpm,
  withRoleModelInText,
  withRoleNameInText,
  withRoleOneMInText,
} from "@/features/providers/derive"
import { GEMINI_PROVIDER_PRESETS } from "@/features/providers/gemini-presets"
import { GROK_PROVIDER_PRESETS } from "@/features/providers/grok-presets"
import { PROVIDER_PRESETS } from "@/features/providers/presets"
import type { Provider } from "@/types/generated/bindings"

/** A provider whose settingsConfig carries a full env block. */
function provider(config: string): Provider {
  return {
    ...emptyProvider(),
    settingsConfig: config,
  }
}

/** The env block of a settingsConfig JSON text, for assertions. Accepts null
 *  so role-write results (null when no model is filled) can be fed straight
 *  in — a null fails loudly here instead of at a less legible parse. */
function envOf(configText: string | null): Record<string, string> {
  if (configText === null) {
    throw new Error("envOf: expected a settingsConfig text, got null")
  }
  return (JSON.parse(configText) as { env: Record<string, string> }).env
}

/** A settingsConfig text with just an env block. */
function configWith(env: Record<string, string>): string {
  return JSON.stringify({ env })
}

describe("providerEndpoint / providerApiKey / providerModel", () => {
  it("reads the basic fields out of the settingsConfig env block", () => {
    const p = provider(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: "https://api.moonshot.cn/anthropic",
          ANTHROPIC_AUTH_TOKEN: "sk-abc",
          ANTHROPIC_MODEL: "kimi-k2.7-code",
        },
      }),
    )
    expect(providerEndpoint(p)).toBe("https://api.moonshot.cn/anthropic")
    expect(providerApiKey(p)).toBe("sk-abc")
    expect(providerModel(p)).toBe("kimi-k2.7-code")
  })

  it("reads the API key from the legacy ANTHROPIC_API_KEY spelling", () => {
    const p = provider(
      JSON.stringify({ env: { ANTHROPIC_API_KEY: "sk-legacy" } }),
    )
    expect(providerApiKey(p)).toBe("sk-legacy")
  })

  it("prefers AUTH_TOKEN over API_KEY when both are present", () => {
    const p = provider(
      JSON.stringify({
        env: { ANTHROPIC_AUTH_TOKEN: "sk-new", ANTHROPIC_API_KEY: "sk-old" },
      }),
    )
    expect(providerApiKey(p)).toBe("sk-new")
  })

  it("returns empty strings for a missing field / garbage / empty config", () => {
    expect(providerEndpoint(provider("{}"))).toBe("")
    expect(providerApiKey(provider("{}"))).toBe("")
    expect(providerModel(provider("not-json"))).toBe("")
    expect(providerEndpoint(provider(""))).toBe("")
    expect(providerApiKey(provider('"a bare string"'))).toBe("")
  })

  it("treats a non-object env (hand-edited garbage) as empty", () => {
    expect(providerEndpoint(provider(JSON.stringify({ env: "nope" })))).toBe("")
    expect(providerApiKey(provider(JSON.stringify({ env: [1, 2] })))).toBe("")
  })
})

describe("withBasicFields", () => {
  it("updates the endpoint/key and preserves the rest of the snapshot", () => {
    const p = provider(
      JSON.stringify({
        includeCoAuthoredBy: false,
        env: {
          ANTHROPIC_BASE_URL: "old-url",
          ANTHROPIC_AUTH_TOKEN: "old-key",
          ANTHROPIC_MODEL: "keep-me",
        },
      }),
    )
    const next = withBasicFields(p, {
      endpoint: "new-url",
      apiKey: "new-key",
    })
    expect(providerEndpoint(next)).toBe("new-url")
    expect(providerApiKey(next)).toBe("new-key")
    expect(providerModel(next)).toBe("keep-me")
    // Non-env settings survive untouched.
    expect(JSON.parse(next.settingsConfig)).toMatchObject({
      includeCoAuthoredBy: false,
    })
  })

  it("clears the endpoint/key when the form fields are empty", () => {
    const p = provider(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: "old-url",
          ANTHROPIC_AUTH_TOKEN: "old-key",
          ANTHROPIC_API_KEY: "legacy-key",
        },
      }),
    )
    const next = withBasicFields(p, { endpoint: "", apiKey: "" })
    expect(providerEndpoint(next)).toBe("")
    expect(providerApiKey(next)).toBe("")
    // Both key spellings are gone.
    const env = (JSON.parse(next.settingsConfig) as { env: object }).env
    expect(env).toEqual({})
  })

  it("migrates a legacy API_KEY to AUTH_TOKEN and drops the old spelling", () => {
    // Editing a provider that only carried ANTHROPIC_API_KEY: the new key is
    // written as AUTH_TOKEN and the old spelling removed, so the snapshot ends
    // with one credential instead of two.
    const p = provider(
      JSON.stringify({ env: { ANTHROPIC_API_KEY: "sk-legacy" } }),
    )
    const next = withBasicFields(p, { endpoint: "", apiKey: "sk-new" })
    expect(providerApiKey(next)).toBe("sk-new")
    const env = (
      JSON.parse(next.settingsConfig) as {
        env: Record<string, string>
      }
    ).env
    expect(env).toEqual({ ANTHROPIC_AUTH_TOKEN: "sk-new" })
  })

  it("keeps the id / name / category of the original provider", () => {
    const p: Provider = {
      ...emptyProvider(),
      id: "p1",
      name: "Kimi",
      category: "cn_official",
    }
    const next = withBasicFields(p, { endpoint: "u", apiKey: "k" })
    expect(next.id).toBe("p1")
    expect(next.name).toBe("Kimi")
    expect(next.category).toBe("cn_official")
  })

  it("tolerates an empty settingsConfig", () => {
    const next = withBasicFields(emptyProvider(), {
      endpoint: "https://x.dev",
      apiKey: "sk-x",
    })
    expect(providerEndpoint(next)).toBe("https://x.dev")
    expect(providerApiKey(next)).toBe("sk-x")
  })
})

describe("emptyProvider", () => {
  it("is a blank custom provider with an empty env and no id", () => {
    const p = emptyProvider()
    expect(p.id).toBe("")
    expect(p.category).toBe("custom")
    expect(providerEndpoint(p)).toBe("")
    expect(providerApiKey(p)).toBe("")
  })
})

describe("configEndpoint / configApiKey", () => {
  it("reads the env-backed fields straight from a JSON text", () => {
    const text = JSON.stringify({
      env: {
        ANTHROPIC_BASE_URL: "https://api.x.dev",
        ANTHROPIC_AUTH_TOKEN: "sk-x",
      },
    })
    expect(configEndpoint(text)).toBe("https://api.x.dev")
    expect(configApiKey(text)).toBe("sk-x")
  })

  it("reads the legacy API_KEY spelling from a JSON text", () => {
    expect(
      configApiKey(JSON.stringify({ env: { ANTHROPIC_API_KEY: "sk-l" } })),
    ).toBe("sk-l")
  })

  it("returns empty strings for garbage / empty text", () => {
    expect(configEndpoint("not-json")).toBe("")
    expect(configApiKey("")).toBe("")
    expect(configEndpoint('"a bare string"')).toBe("")
  })
})

describe("withBasicFieldsInText", () => {
  it("is the text-level twin of withBasicFields", () => {
    const text = JSON.stringify({
      includeCoAuthoredBy: false,
      env: {
        ANTHROPIC_BASE_URL: "old-url",
        ANTHROPIC_AUTH_TOKEN: "old-key",
        ANTHROPIC_MODEL: "keep-me",
      },
    })
    const next = withBasicFieldsInText(text, {
      endpoint: "new-url",
      apiKey: "new-key",
    })
    expect(configEndpoint(next)).toBe("new-url")
    expect(configApiKey(next)).toBe("new-key")
    expect(JSON.parse(next)).toMatchObject({
      includeCoAuthoredBy: false,
      env: { ANTHROPIC_MODEL: "keep-me" },
    })
  })

  it("formats the result with 2-space indentation", () => {
    const next = withBasicFieldsInText('{"env":{"ANTHROPIC_BASE_URL":"u"}}', {
      endpoint: "",
      apiKey: "",
    })
    expect(next).toBe('{\n  "env": {}\n}')
  })
})

describe("providerFromPreset", () => {
  it("builds a custom-category draft that copies the preset snapshot verbatim", () => {
    const kimi = PROVIDER_PRESETS.find((p) => p.name === "Kimi")
    expect(kimi).toBeDefined()
    const draft = providerFromPreset(kimi!)
    // 预设是起点、定制是终点：落表单的草稿归 custom，id 留空让 save 分配。
    expect(draft.id).toBe("")
    expect(draft.category).toBe("custom")
    expect(draft.name).toBe("Kimi")
    expect(draft.settingsConfig).toBe(kimi!.settingsConfig)
    // derive 读函数直接回填表单字段，无需另起一套解析。
    expect(providerEndpoint(draft)).toBe("https://api.moonshot.cn/anthropic")
    expect(providerModel(draft)).toBe("kimi-k2.7-code")
    expect(providerApiKey(draft)).toBe("")
  })

  it("keeps the preset's model mapping after withBasicFields writes the form fields", () => {
    const glm = PROVIDER_PRESETS.find((p) => p.name === "Zhipu GLM")
    expect(glm).toBeDefined()
    const next = withBasicFields(providerFromPreset(glm!), {
      endpoint: "https://example.com/anthropic",
      apiKey: "sk-123",
    })
    expect(providerEndpoint(next)).toBe("https://example.com/anthropic")
    expect(providerApiKey(next)).toBe("sk-123")
    const env = (
      JSON.parse(next.settingsConfig) as {
        env: Record<string, string>
      }
    ).env
    // 模型映射（表单不拥有的字段）原样保留。
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_MODEL).toBe("glm-5.1")
  })

  it("keeps the Bedrock template-variable placeholders until the template step", () => {
    const bedrock = PROVIDER_PRESETS.find(
      (p) => p.name === "AWS Bedrock (AKSK)",
    )
    expect(bedrock).toBeDefined()
    const draft = providerFromPreset(bedrock!)
    const env = (
      JSON.parse(draft.settingsConfig) as {
        env: Record<string, string>
      }
    ).env
    expect(env.ANTHROPIC_BASE_URL).toBe(
      // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
      "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
    )
    expect(env.AWS_ACCESS_KEY_ID).toBe(
      // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
      "${AWS_ACCESS_KEY_ID}",
    )
    expect(env.CLAUDE_CODE_USE_BEDROCK).toBe("1")
  })

  it("never mutates the preset constant", () => {
    const before = JSON.stringify(PROVIDER_PRESETS)
    const draft = providerFromPreset(PROVIDER_PRESETS[0]!)
    expect(draft.settingsConfig).toBe(PROVIDER_PRESETS[0]!.settingsConfig)
    expect(JSON.stringify(PROVIDER_PRESETS)).toBe(before)
  })
})

describe("1M marker helpers", () => {
  it("hasOneM detects the marker case-insensitively", () => {
    expect(hasOneM("claude-sonnet-5[1M]")).toBe(true)
    // 代理转发上游时写小写标记，读端两种拼写都要认得。
    expect(hasOneM("claude-sonnet-5[1m]")).toBe(true)
    expect(hasOneM("claude-sonnet-5")).toBe(false)
    expect(hasOneM("")).toBe(false)
  })

  it("stripOneM removes a trailing marker and nothing else", () => {
    expect(stripOneM("claude-sonnet-5[1M]")).toBe("claude-sonnet-5")
    expect(stripOneM("claude-sonnet-5[1m]")).toBe("claude-sonnet-5")
    expect(stripOneM("claude-sonnet-5 [1M]")).toBe("claude-sonnet-5")
    expect(stripOneM("claude-sonnet-5")).toBe("claude-sonnet-5")
    // 只剥最末尾的一个标记，中间出现的不动。
    expect(stripOneM("claude-sonnet-5[1M][1M]")).toBe("claude-sonnet-5[1M]")
    expect(stripOneM("claude-sonnet-5[1M]-x")).toBe("claude-sonnet-5[1M]-x")
    expect(stripOneM("[1M]")).toBe("")
  })

  it("setModelOneM appends and idempotently strips before re-applying", () => {
    expect(setModelOneM("claude-sonnet-5", true)).toBe("claude-sonnet-5[1M]")
    expect(setModelOneM("claude-sonnet-5[1M]", true)).toBe(
      "claude-sonnet-5[1M]",
    )
    expect(setModelOneM("claude-sonnet-5[1M]", false)).toBe("claude-sonnet-5")
    expect(setModelOneM("claude-sonnet-5", false)).toBe("claude-sonnet-5")
    expect(setModelOneM("", true)).toBe("")
    expect(setModelOneM("  ", false)).toBe("")
  })
})

describe("configRoleModel backfill chain", () => {
  it("prefers the role's own key over any backfill", () => {
    const text = configWith({
      ANTHROPIC_MODEL: "fallback",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
    })
    expect(configRoleModel(text, "sonnet")).toBe("glm-5.1")
  })

  it("falls back to the primary model for sonnet and opus", () => {
    const text = configWith({ ANTHROPIC_MODEL: "glm-5.1" })
    expect(configRoleModel(text, "sonnet")).toBe("glm-5.1")
    expect(configRoleModel(text, "opus")).toBe("glm-5.1")
  })

  it("falls back to the legacy small-fast key for haiku, then the primary model", () => {
    expect(
      configRoleModel(
        configWith({ ANTHROPIC_SMALL_FAST_MODEL: "glm-5-flash" }),
        "haiku",
      ),
    ).toBe("glm-5-flash")
    expect(
      configRoleModel(configWith({ ANTHROPIC_MODEL: "glm-5.1" }), "haiku"),
    ).toBe("glm-5.1")
    expect(configRoleModel(configWith({}), "haiku")).toBe("")
  })

  it("fable falls back to the opus role key, then the primary model", () => {
    expect(
      configRoleModel(
        configWith({ ANTHROPIC_DEFAULT_OPUS_MODEL: "claude-opus-5" }),
        "fable",
      ),
    ).toBe("claude-opus-5")
    expect(
      configRoleModel(configWith({ ANTHROPIC_MODEL: "glm-5.1" }), "fable"),
    ).toBe("glm-5.1")
    // Fable 不会回填到其他角色的键。
    expect(
      configRoleModel(
        configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1" }),
        "fable",
      ),
    ).toBe("")
  })

  it("subagent falls back to the sonnet role key, then the primary model", () => {
    expect(
      configRoleModel(
        configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1" }),
        "subagent",
      ),
    ).toBe("glm-5.1")
    expect(
      configRoleModel(configWith({ ANTHROPIC_MODEL: "glm-5.1" }), "subagent"),
    ).toBe("glm-5.1")
    expect(configRoleModel(configWith({}), "subagent")).toBe("")
  })

  it("returns the raw env value, 1M marker included", () => {
    expect(
      configRoleModel(configWith({ ANTHROPIC_MODEL: "glm-5.1[1M]" }), "sonnet"),
    ).toBe("glm-5.1[1M]")
  })

  it("returns empty strings for garbage / empty config", () => {
    expect(configRoleModel("not-json", "sonnet")).toBe("")
    expect(configRoleModel("", "fable")).toBe("")
    expect(configRoleModel('{"env": [1, 2]}', "opus")).toBe("")
  })

  it("reads role models out of preset snapshots (production path)", () => {
    const kimi = PROVIDER_PRESETS.find((p) => p.name === "Kimi")
    const draft = providerFromPreset(kimi!)
    expect(configRoleModel(draft.settingsConfig, "sonnet")).toBe(
      "kimi-k2.7-code",
    )
    expect(configRoleModel(draft.settingsConfig, "haiku")).toBe(
      "kimi-k2.7-code",
    )
    // Fable 经 Opus 角色键回填，Subagent 经 Sonnet 角色键回填。
    expect(configRoleModel(draft.settingsConfig, "fable")).toBe(
      "kimi-k2.7-code",
    )
    expect(configRoleModel(draft.settingsConfig, "subagent")).toBe(
      "kimi-k2.7-code",
    )
    const deepseek = PROVIDER_PRESETS.find((p) => p.name === "DeepSeek")
    const ds = providerFromPreset(deepseek!)
    // Haiku / Sonnet 角色键与主模型不同，回填必须取到各自的值。
    expect(configRoleModel(ds.settingsConfig, "haiku")).toBe(
      "deepseek-v4-flash",
    )
    expect(configRoleModel(ds.settingsConfig, "subagent")).toBe(
      "deepseek-v4-pro",
    )
  })
})

describe("configRoleName / configRoleFields", () => {
  it("prefers the _NAME key over the model name", () => {
    const text = configWith({
      ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "我的主力",
    })
    expect(configRoleName(text, "sonnet")).toBe("我的主力")
  })

  it("defaults to the marker-free model name", () => {
    expect(
      configRoleName(configWith({ ANTHROPIC_MODEL: "glm-5.1[1M]" }), "sonnet"),
    ).toBe("glm-5.1")
    expect(configRoleName(configWith({}), "sonnet")).toBe("")
  })

  it("configRoleFields returns model / name / oneM together", () => {
    expect(
      configRoleFields(
        configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1[1M]" }),
        "sonnet",
      ),
    ).toEqual({ model: "glm-5.1[1M]", name: "glm-5.1", oneM: true })
  })

  it("haiku never reports oneM even when its model carries a stray marker", () => {
    expect(
      configRoleHasOneM(
        configWith({ ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-5-flash[1M]" }),
        "haiku",
      ),
    ).toBe(false)
  })

  it("oneM derives from a backfilled model too", () => {
    expect(
      configRoleHasOneM(configWith({ ANTHROPIC_MODEL: "glm-5.1[1M]" }), "opus"),
    ).toBe(true)
  })
})

describe("withRoleModelInText", () => {
  it("writes the role model and preserves the rest of the snapshot", () => {
    const next = withRoleModelInText(
      JSON.stringify({
        includeCoAuthoredBy: false,
        env: { ANTHROPIC_MODEL: "glm-5.1" },
      }),
      "sonnet",
      "glm-5.2",
    )
    expect(JSON.parse(next)).toMatchObject({
      includeCoAuthoredBy: false,
      env: {
        ANTHROPIC_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.2",
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "glm-5.2",
      },
    })
  })

  it("syncs the display name when it equals the old model name", () => {
    const next = withRoleModelInText(
      configWith({
        ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_OPUS_MODEL_NAME: "glm-5.1",
      }),
      "opus",
      "glm-5.2",
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("glm-5.2")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME).toBe("glm-5.2")
  })

  it("writes the marker-free default display name when the key is absent", () => {
    const next = withRoleModelInText(
      configWith({ ANTHROPIC_MODEL: "glm-5.1" }),
      "sonnet",
      "glm-5.2[1M]",
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.2[1M]")
    // 显示名跟随剥掉标记的模型名，不带 [1M]。
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("glm-5.2")
  })

  it("keeps a custom display name untouched", () => {
    const next = withRoleModelInText(
      configWith({
        ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "主力模型",
      }),
      "sonnet",
      "glm-5.2",
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.2")
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("主力模型")
  })

  it("keeps the marker for 1M-capable roles", () => {
    const next = withRoleModelInText(configWith({}), "sonnet", "glm-5.1[1M]")
    expect(envOf(next).ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1[1M]")
  })

  it("strips the 1M marker for haiku on write", () => {
    const next = withRoleModelInText(configWith({}), "haiku", "glm-5-flash[1M]")
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-5-flash")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME).toBe("glm-5-flash")
  })

  it("deletes the legacy small-fast key on every write", () => {
    const next = withRoleModelInText(
      configWith({
        ANTHROPIC_SMALL_FAST_MODEL: "glm-5-flash",
        ANTHROPIC_MODEL: "glm-5.1",
      }),
      "sonnet",
      "glm-5.2",
    )
    expect(envOf(next).ANTHROPIC_SMALL_FAST_MODEL).toBeUndefined()
  })

  it("an empty model clears the key and a synced display name", () => {
    const next = withRoleModelInText(
      configWith({
        ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "glm-5.1",
      }),
      "sonnet",
      "",
    )
    expect(envOf(next)).toEqual({})
  })

  it("an empty model keeps a custom display name", () => {
    const next = withRoleModelInText(
      configWith({
        ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "主力模型",
      }),
      "sonnet",
      "",
    )
    expect(envOf(next)).toEqual({
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "主力模型",
    })
  })
})

describe("withRoleNameInText", () => {
  it("writes the display name key and clears it when emptied", () => {
    const written = withRoleNameInText(configWith({}), "sonnet", "我的主力")
    expect(envOf(written).ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("我的主力")
    const cleared = withRoleNameInText(written, "sonnet", "")
    expect(envOf(cleared)).toEqual({})
  })
})

describe("withRoleOneMInText", () => {
  it("checking appends the marker and unchecking strips it", () => {
    const checked = withRoleOneMInText(
      configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1" }),
      "sonnet",
      true,
    )
    expect(envOf(checked).ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1[1M]")
    const unchecked = withRoleOneMInText(checked, "sonnet", false)
    expect(envOf(unchecked).ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1")
  })

  it("keeps the display name marker-free while toggling", () => {
    const checked = withRoleOneMInText(
      configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1" }),
      "sonnet",
      true,
    )
    const env = envOf(checked)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1[1M]")
  })

  it("writes the marker into the role key when the model only comes from backfill", () => {
    const checked = withRoleOneMInText(
      configWith({ ANTHROPIC_MODEL: "glm-5.1" }),
      "sonnet",
      true,
    )
    const env = envOf(checked)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("glm-5.1")
    // 主模型键不受影响。
    expect(env.ANTHROPIC_MODEL).toBe("glm-5.1")
  })

  it("is a no-op for haiku", () => {
    const text = configWith({ ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-5-flash" })
    expect(withRoleOneMInText(text, "haiku", true)).toBe(text)
  })
})

describe("withAllRolesFromFirstInText", () => {
  it("applies the primary model to every role with synced display names", () => {
    const next = withAllRolesFromFirstInText(
      configWith({ ANTHROPIC_MODEL: "glm-5.1" }),
    )
    expect(envOf(next)).toEqual({
      ANTHROPIC_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "glm-5.1",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_OPUS_MODEL_NAME: "glm-5.1",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME: "glm-5.1",
      ANTHROPIC_DEFAULT_FABLE_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_FABLE_MODEL_NAME: "glm-5.1",
      ANTHROPIC_DEFAULT_SUBAGENT_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_SUBAGENT_MODEL_NAME: "glm-5.1",
    })
  })

  it("propagates the 1M marker to capable roles and strips it for haiku", () => {
    const next = withAllRolesFromFirstInText(
      configWith({ ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.1[1M]" }),
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_SUBAGENT_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-5.1")
    // 显示名一律不带标记。
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME).toBe("glm-5.1")
  })

  it("picks the first filled role when the primary model is absent", () => {
    const next = withAllRolesFromFirstInText(
      configWith({ ANTHROPIC_DEFAULT_FABLE_MODEL: "glm-5.1" }),
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_SUBAGENT_MODEL).toBe("glm-5.1")
  })

  it("prefers an earlier role over a later one", () => {
    const next = withAllRolesFromFirstInText(
      configWith({
        ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-5-flash",
      }),
    )
    expect(envOf(next).ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-5.1")
  })

  it("deletes the legacy small-fast key and preserves the rest of the snapshot", () => {
    const next = withAllRolesFromFirstInText(
      JSON.stringify({
        includeCoAuthoredBy: false,
        env: {
          ANTHROPIC_SMALL_FAST_MODEL: "glm-5-flash",
          ANTHROPIC_MODEL: "glm-5.1",
        },
      }),
    )
    expect(next).not.toBeNull()
    const parsed = JSON.parse(next as string) as {
      includeCoAuthoredBy: boolean
      env: Record<string, string>
    }
    expect(parsed.includeCoAuthoredBy).toBe(false)
    expect(parsed.env.ANTHROPIC_SMALL_FAST_MODEL).toBeUndefined()
  })

  it("returns null when no model is filled anywhere", () => {
    expect(withAllRolesFromFirstInText(configWith({}))).toBeNull()
    expect(
      withAllRolesFromFirstInText(
        configWith({ ANTHROPIC_BASE_URL: "https://x.dev" }),
      ),
    ).toBeNull()
    expect(withAllRolesFromFirstInText("not-json")).toBeNull()
  })

  it("ignores whitespace-only env values when picking", () => {
    expect(
      withAllRolesFromFirstInText(
        configWith({
          ANTHROPIC_MODEL: "  ",
          ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.1",
        }),
      ),
    ).not.toBeNull()
  })
})

describe("withAllRolesInText", () => {
  it("writes the given model to every role with display-name sync", () => {
    const next = withAllRolesInText(configWith({}), "my-model[1M]")
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("my-model[1M]")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("my-model[1M]")
    expect(env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe("my-model[1M]")
    expect(env.ANTHROPIC_DEFAULT_SUBAGENT_MODEL).toBe("my-model[1M]")
    // Haiku 不支持 1M 标记，写入时剥离；显示名一律不带标记。
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME).toBe("my-model")
  })

  it("preserves a hand-set display name that differs from the model", () => {
    const next = withAllRolesInText(
      configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "My Favorite" }),
      "glm-5.1",
    )
    expect(envOf(next).ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("My Favorite")
  })
})
describe("configAuthField / switchAuthField", () => {
  it("maps the field names to the env keys", () => {
    expect(authFieldKey("auth_token")).toBe("ANTHROPIC_AUTH_TOKEN")
    expect(authFieldKey("api_key")).toBe("ANTHROPIC_API_KEY")
  })

  it("defaults to AUTH_TOKEN when no key, or only AUTH_TOKEN, is present", () => {
    expect(configAuthField("{}")).toBe("auth_token")
    expect(configAuthField(JSON.stringify({ env: {} }))).toBe("auth_token")
    expect(
      configAuthField(JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: "" } })),
    ).toBe("auth_token")
  })

  it("reports API_KEY when that is the only spelling present", () => {
    expect(
      configAuthField(JSON.stringify({ env: { ANTHROPIC_API_KEY: "sk" } })),
    ).toBe("api_key")
  })

  it("prefers AUTH_TOKEN when both spellings are present", () => {
    expect(
      configAuthField(
        JSON.stringify({
          env: { ANTHROPIC_AUTH_TOKEN: "a", ANTHROPIC_API_KEY: "b" },
        }),
      ),
    ).toBe("auth_token")
  })

  it("moves the key value to the target field and deletes the old key", () => {
    const next = switchAuthField(
      JSON.stringify({
        includeCoAuthoredBy: false,
        env: { ANTHROPIC_AUTH_TOKEN: "sk-x", ANTHROPIC_MODEL: "keep-me" },
      }),
      "auth_token",
      "api_key",
    )
    expect(JSON.parse(next)).toEqual({
      includeCoAuthoredBy: false,
      env: { ANTHROPIC_API_KEY: "sk-x", ANTHROPIC_MODEL: "keep-me" },
    })
  })

  it("moves the legacy API_KEY spelling back to AUTH_TOKEN", () => {
    const next = switchAuthField(
      JSON.stringify({ env: { ANTHROPIC_API_KEY: "sk-legacy" } }),
      "api_key",
      "auth_token",
    )
    expect(JSON.parse(next)).toEqual({
      env: { ANTHROPIC_AUTH_TOKEN: "sk-legacy" },
    })
  })

  it("moves an empty value so the selected field survives a toggled preset", () => {
    const next = switchAuthField(
      JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: "" } }),
      "auth_token",
      "api_key",
    )
    expect(JSON.parse(next)).toEqual({ env: { ANTHROPIC_API_KEY: "" } })
    expect(configAuthField(next)).toBe("api_key")
  })

  it("removes the old key without creating a new one when there is no value", () => {
    const next = switchAuthField('{"env":{}}', "auth_token", "api_key")
    expect(JSON.parse(next)).toEqual({ env: {} })
  })

  it("is a no-op when from === to", () => {
    const text = JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: "sk" } })
    expect(switchAuthField(text, "auth_token", "auth_token")).toBe(text)
  })
})

describe("withBasicFields with a selected auth field", () => {
  it("writes the key under the selected field and drops the other spelling", () => {
    const p = provider(
      JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: "sk-old" } }),
    )
    const next = withBasicFields(p, {
      endpoint: "",
      apiKey: "sk-new",
      authField: "api_key",
    })
    expect(providerApiKey(next)).toBe("sk-new")
    const env = (
      JSON.parse(next.settingsConfig) as {
        env: Record<string, string>
      }
    ).env
    expect(env).toEqual({ ANTHROPIC_API_KEY: "sk-new" })
  })

  it("is the text-level twin: an API_KEY selection survives a field merge", () => {
    const next = withBasicFieldsInText(
      '{"env":{"ANTHROPIC_AUTH_TOKEN":"old"}}',
      { endpoint: "https://x.dev", apiKey: "k", authField: "api_key" },
    )
    expect(JSON.parse(next)).toEqual({
      env: { ANTHROPIC_BASE_URL: "https://x.dev", ANTHROPIC_API_KEY: "k" },
    })
    expect(configAuthField(next)).toBe("api_key")
  })
})

describe("extractTemplateVars / replaceTemplateVarsInText", () => {
  it("collects the Bedrock preset's placeholder names, deduped, in order", () => {
    const bedrock = PROVIDER_PRESETS.find(
      (p) => p.name === "AWS Bedrock (AKSK)",
    )
    expect(bedrock).toBeDefined()
    expect(extractTemplateVars(bedrock!.settingsConfig)).toEqual([
      "AWS_REGION",
      "AWS_ACCESS_KEY_ID",
      "AWS_SECRET_ACCESS_KEY",
    ])
  })

  it("finds and replaces placeholders in nested non-env settings too", () => {
    const text = JSON.stringify({
      env: { ANTHROPIC_BASE_URL: "https://x.dev" },
      hooks: {
        PreToolUse: [
          {
            // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
            matcher: "http://${HOST}/cb",
          },
        ],
      },
    })
    expect(extractTemplateVars(text)).toEqual(["HOST"])
    const next = replaceTemplateVarsInText(text, { HOST: "127.0.0.1" })
    expect(JSON.parse(next)).toEqual({
      env: { ANTHROPIC_BASE_URL: "https://x.dev" },
      hooks: { PreToolUse: [{ matcher: "http://127.0.0.1/cb" }] },
    })
  })

  it("returns an empty list for a clean or garbage snapshot", () => {
    expect(extractTemplateVars("{}")).toEqual([])
    expect(extractTemplateVars("not-json")).toEqual([])
  })

  it("replaces filled variables and keeps the unfilled placeholders verbatim", () => {
    const bedrock = PROVIDER_PRESETS.find(
      (p) => p.name === "AWS Bedrock (AKSK)",
    )
    expect(bedrock).toBeDefined()
    const next = replaceTemplateVarsInText(bedrock!.settingsConfig, {
      AWS_REGION: "ap-northeast-1",
    })
    const env = (
      JSON.parse(next) as {
        env: Record<string, string>
      }
    ).env
    expect(env.ANTHROPIC_BASE_URL).toBe(
      "https://bedrock-runtime.ap-northeast-1.amazonaws.com",
    )
    expect(env.AWS_REGION).toBe("ap-northeast-1")
    expect(env.AWS_ACCESS_KEY_ID).toBe(
      // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
      "${AWS_ACCESS_KEY_ID}",
    )
    expect(extractTemplateVars(next)).toEqual([
      "AWS_ACCESS_KEY_ID",
      "AWS_SECRET_ACCESS_KEY",
    ])
  })

  it("keeps the placeholder verbatim when a variable is missing or empty", () => {
    const text = JSON.stringify({
      env: {
        // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
        ANTHROPIC_BASE_URL:
          "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
      },
    })
    expect(
      JSON.parse(replaceTemplateVarsInText(text, { AWS_REGION: "" })),
    ).toEqual(JSON.parse(text))
    expect(JSON.parse(replaceTemplateVarsInText(text, {}))).toEqual(
      JSON.parse(text),
    )
  })
})

describe("restoreTemplatePlaceholders", () => {
  it("reverts every occurrence of a recorded value to its placeholder", () => {
    const text = JSON.stringify({
      env: {
        ANTHROPIC_BASE_URL: "https://bedrock-runtime.us-east-1.amazonaws.com",
        AWS_REGION: "us-east-1",
      },
    })
    const restored = restoreTemplatePlaceholders(text, {
      AWS_REGION: "us-east-1",
    })
    expect(JSON.parse(restored)).toEqual({
      env: {
        // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
        ANTHROPIC_BASE_URL:
          "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
        // biome-ignore lint/suspicious/noTemplateCurlyInString: 断言字面量占位符文本
        AWS_REGION: "${AWS_REGION}",
      },
    })
  })

  it("leaves values without a recorded template untouched", () => {
    const text = JSON.stringify({
      env: { ANTHROPIC_BASE_URL: "https://x.dev" },
    })
    expect(
      JSON.parse(
        restoreTemplatePlaceholders(text, { AWS_REGION: "us-east-1" }),
      ),
    ).toEqual(JSON.parse(text))
  })
})

describe("metaTemplateValues / withMetaTemplateValues", () => {
  it("reads the recorded values; garbage or empty meta → {}", () => {
    const meta = withMetaTemplateValues("{}", {
      AWS_REGION: "ap-1",
      AWS_ACCESS_KEY_ID: "AKIA1",
    })
    expect(metaTemplateValues(meta)).toEqual({
      AWS_REGION: "ap-1",
      AWS_ACCESS_KEY_ID: "AKIA1",
    })
    expect(metaTemplateValues("not-json")).toEqual({})
    expect(metaTemplateValues("")).toEqual({})
  })

  it("keeps unknown meta keys and drops non-string entries", () => {
    const meta = withMetaTemplateValues('{"favorite": true}', {
      AWS_REGION: "ap-1",
    })
    expect(JSON.parse(meta)).toEqual({
      favorite: true,
      templateValues: { AWS_REGION: "ap-1" },
    })
    expect(
      metaTemplateValues('{"templateValues": {"A": "x", "B": 3}}'),
    ).toEqual({ A: "x" })
  })

  it("removes the templateValues key when nothing is filled", () => {
    expect(withMetaTemplateValues("{}", {})).toBe("{}")
    expect(
      withMetaTemplateValues('{"templateValues": {"A": "old"}}', { A: "" }),
    ).toBe("{}")
  })
})

describe("providerLiveManaged / providerLiveKey", () => {
  // 附加模式（opencode）的 live 状态：liveManaged 严格 true 判定、liveKey 严格
  // 字符串判定——meta 里的非布尔 / 非字符串垃圾值都不算，与 parseMeta 的宽容
  // 契约一致（坏 meta 不抛、归一为默认）。
  const live = (meta: string) => ({ ...emptyProvider("opencode"), meta })

  it("liveManaged: true only on a boolean true; everything else → false", () => {
    expect(providerLiveManaged(live('{"liveManaged": true}'))).toBe(true)
    expect(providerLiveManaged(live('{"liveManaged": false}'))).toBe(false)
    expect(providerLiveManaged(live("{}"))).toBe(false)
    expect(providerLiveManaged(live("not-json"))).toBe(false)
    expect(providerLiveManaged(live(""))).toBe(false)
    // 非布尔垃圾值不算（"true" 字符串、数字 1 都不是 true）。
    expect(providerLiveManaged(live('{"liveManaged": "true"}'))).toBe(false)
    expect(providerLiveManaged(live('{"liveManaged": 1}'))).toBe(false)
  })

  it("liveKey: the string key; non-string / missing / garbage → empty", () => {
    expect(providerLiveKey(live('{"liveKey": "my-provider"}'))).toBe(
      "my-provider",
    )
    expect(providerLiveKey(live("{}"))).toBe("")
    expect(providerLiveKey(live('{"liveKey": 123}'))).toBe("")
    expect(providerLiveKey(live(""))).toBe("")
    expect(providerLiveKey(live("not-json"))).toBe("")
  })
})

describe("Bedrock preset end to end", () => {
  const bedrock = PROVIDER_PRESETS.find((p) => p.name === "AWS Bedrock (AKSK)")!
  const values = {
    AWS_REGION: "ap-northeast-1",
    AWS_ACCESS_KEY_ID: "AKIAEXAMPLEKEY",
    AWS_SECRET_ACCESS_KEY: "SKEXAMPLE",
  }

  it("materializes the preset placeholders into the snapshot", () => {
    const next = replaceTemplateVarsInText(bedrock.settingsConfig, values)
    expect(extractTemplateVars(next)).toEqual([])
    const env = (
      JSON.parse(next) as {
        env: Record<string, string>
      }
    ).env
    expect(env.ANTHROPIC_BASE_URL).toBe(
      "https://bedrock-runtime.ap-northeast-1.amazonaws.com",
    )
    expect(env.AWS_REGION).toBe("ap-northeast-1")
    expect(env.AWS_ACCESS_KEY_ID).toBe("AKIAEXAMPLEKEY")
    expect(env.AWS_SECRET_ACCESS_KEY).toBe("SKEXAMPLE")
    // 非模板字段原样保留。
    expect(env.CLAUDE_CODE_USE_BEDROCK).toBe("1")
  })

  it("restores the placeholders verbatim for re-editing from meta values", () => {
    const materialized = replaceTemplateVarsInText(
      bedrock.settingsConfig,
      values,
    )
    expect(restoreTemplatePlaceholders(materialized, values)).toBe(
      bedrock.settingsConfig,
    )
  })
})
describe("snippetMissingKeys", () => {
  const snippet = JSON.stringify({ includeCoAuthoredBy: false })

  it("reports controlled keys the config does not have", () => {
    // 配置里没有 includeCoAuthoredBy → 片段会在写盘时补上它。
    expect(snippetMissingKeys('{"env":{}}', snippet)).toEqual([
      "includeCoAuthoredBy",
    ])
  })

  it("reports env only when at least one snippet env key is missing", () => {
    const cfg = JSON.stringify({ env: { ANTHROPIC_MODEL: "m" } })
    expect(
      snippetMissingKeys(cfg, JSON.stringify({ env: { A: "1" } })),
    ).toEqual(["env"])
    // 片段 env 全被配置覆盖 → 空。
    expect(
      snippetMissingKeys(
        cfg,
        JSON.stringify({ env: { ANTHROPIC_MODEL: "snippet-default" } }),
      ),
    ).toEqual([])
  })

  it("ignores snippet keys that are not controlled fields", () => {
    const bad = JSON.stringify({
      permissions: { deny: ["Bash"] },
      hooks: { PostToolUse: [] },
      model: "claude-opus-4-5",
    })
    expect(snippetMissingKeys('{"env":{}}', bad)).toEqual([])
  })

  it("reports only the keys actually missing (subset)", () => {
    const cfg = JSON.stringify({ includeCoAuthoredBy: true, env: {} })
    const snip = JSON.stringify({
      includeCoAuthoredBy: false,
      skipWebFetchPreflight: true,
    })
    expect(snippetMissingKeys(cfg, snip)).toEqual(["skipWebFetchPreflight"])
  })

  it("returns empty for empty or garbage input", () => {
    expect(snippetMissingKeys("", "")).toEqual([])
    expect(snippetMissingKeys("{nope", snippet)).toEqual([])
    expect(snippetMissingKeys('{"env":{}}', "{nope")).toEqual([])
  })
})

describe("providerMissingRequired", () => {
  function p(
    config: string,
    category: Provider["category"] = "custom",
  ): Provider {
    return { ...provider(config), category }
  }

  it("reports nothing for a fully configured provider", () => {
    const pv = p(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: "https://api.example.com",
          ANTHROPIC_AUTH_TOKEN: "sk-x",
        },
      }),
    )
    expect(providerMissingRequired(pv)).toEqual([])
  })

  it("reports missing endpoint and API key", () => {
    expect(providerMissingRequired(p('{"env":{}}'))).toEqual([
      "endpoint",
      "apiKey",
    ])
  })

  it("reads the key from either auth spelling (AUTH_TOKEN first)", () => {
    const apiKeyOnly = p(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: "https://x.dev",
          ANTHROPIC_API_KEY: "sk-legacy",
        },
      }),
    )
    expect(providerMissingRequired(apiKeyOnly)).toEqual([])
  })

  it("skips endpoint/key checks for official and cloud-provider presets", () => {
    // Claude Official 走默认端点；Bedrock 用模板变量认证——都不要求端点/key。
    const official = p('{"env":{}}', "official")
    expect(providerMissingRequired(official)).toEqual([])
    const cloud = p('{"env":{}}', "cloud_provider")
    expect(providerMissingRequired(cloud)).toEqual([])
  })

  it("reports unfilled template variables for any category", () => {
    const cloud = p(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL:
            // biome-ignore lint/suspicious/noTemplateCurlyInString: 模板变量占位符
            "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
        },
      }),
      "cloud_provider",
    )
    expect(providerMissingRequired(cloud)).toEqual(["templateVars"])
  })
})

describe("filterProviders", () => {
  const all: Provider[] = [
    { ...provider('{"env":{}}'), name: "Kimi", category: "cn_official" },
    { ...provider('{"env":{}}'), name: "DeepSeek", category: "cn_official" },
    { ...provider('{"env":{}}'), name: "My Custom", category: "custom" },
  ]

  it("returns the list unchanged for an empty query", () => {
    expect(filterProviders(all, "")).toEqual(all)
    expect(filterProviders(all, "   ")).toEqual(all)
  })

  it("matches by name, case-insensitive, contains", () => {
    expect(filterProviders(all, "kimi").map((p) => p.name)).toEqual(["Kimi"])
    expect(filterProviders(all, "CUSTOM").map((p) => p.name)).toEqual([
      "My Custom",
    ])
  })

  it("matches by category identifier", () => {
    expect(filterProviders(all, "cn_official").map((p) => p.name)).toEqual([
      "Kimi",
      "DeepSeek",
    ])
  })

  it("returns empty for no matches", () => {
    expect(filterProviders(all, "zzz")).toEqual([])
  })
})

describe("emptyProvider / providerFromPreset app 参数", () => {
  it("emptyProvider 默认 claude，settingsConfig 形状对齐", () => {
    const p = emptyProvider()
    expect(p.app).toBe("claude")
    expect(p.settingsConfig).toBe('{\n  "env": {}\n}')
  })

  it("emptyProvider('codex') 的 settingsConfig 是 {}", () => {
    const p = emptyProvider("codex")
    expect(p.app).toBe("codex")
    expect(p.settingsConfig).toBe("{}")
  })

  it("emptyProvider('gemini') 的 settingsConfig 是 {}", () => {
    const p = emptyProvider("gemini")
    expect(p.app).toBe("gemini")
    expect(p.settingsConfig).toBe("{}")
  })

  it("providerFromPreset 默认 claude", () => {
    const kimi = PROVIDER_PRESETS.find((p) => p.name === "Kimi")
    const draft = providerFromPreset(kimi!)
    expect(draft.app).toBe("claude")
  })

  it("providerFromPreset 接受 codex，复制 Codex 预设快照", () => {
    const codexKimi = CODEX_PROVIDER_PRESETS.find((p) => p.name === "Kimi")
    expect(codexKimi).toBeDefined()
    const draft = providerFromPreset(codexKimi!, "codex")
    expect(draft.app).toBe("codex")
    expect(draft.name).toBe("Kimi")
    expect(draft.category).toBe("custom")
    expect(draft.settingsConfig).toBe(codexKimi!.settingsConfig)
    expect(codexApiKey(draft.settingsConfig)).toBe("")
    expect(codexConfigToml(draft.settingsConfig)).toContain("kimi-k2.7-code")
  })

  it("providerFromPreset 接受 gemini，复制 Gemini 预设快照", () => {
    const orPreset = GEMINI_PROVIDER_PRESETS.find(
      (p) => p.name === "OpenRouter",
    )
    expect(orPreset).toBeDefined()
    const draft = providerFromPreset(orPreset!, "gemini")
    expect(draft.app).toBe("gemini")
    expect(draft.settingsConfig).toBe(orPreset!.settingsConfig)
    expect(geminiModel(draft.settingsConfig)).toBe("gemini-3.6-flash")
    expect(geminiBaseUrl(draft.settingsConfig)).toBe(
      "https://openrouter.ai/api",
    )
  })

  it("providerFromPreset 不改动预设常量", () => {
    const before = JSON.stringify(CODEX_PROVIDER_PRESETS)
    providerFromPreset(CODEX_PROVIDER_PRESETS[0]!, "codex")
    expect(JSON.stringify(CODEX_PROVIDER_PRESETS)).toBe(before)
  })
})

describe("parseCodexConfig", () => {
  it("解析 auth 与 config（API Key 版）", () => {
    const text = JSON.stringify({
      auth: { OPENAI_API_KEY: "sk-1" },
      config: 'model = "m"',
    })
    expect(parseCodexConfig(text)).toEqual({
      auth: { OPENAI_API_KEY: "sk-1" },
      config: 'model = "m"',
    })
  })

  it("登录态版（auth 为空对象、config 空串）", () => {
    expect(parseCodexConfig("{}")).toEqual({ auth: {}, config: "" })
    expect(parseCodexConfig('{"auth":{}}')).toEqual({ auth: {}, config: "" })
    expect(parseCodexConfig(JSON.stringify({ auth: {}, config: "" }))).toEqual({
      auth: {},
      config: "",
    })
  })

  it('空 / 垃圾 / 非对象 → {auth:{}, config:""}', () => {
    expect(parseCodexConfig("")).toEqual({ auth: {}, config: "" })
    expect(parseCodexConfig("not-json")).toEqual({ auth: {}, config: "" })
    expect(parseCodexConfig("[1,2]")).toEqual({ auth: {}, config: "" })
    expect(parseCodexConfig('"a bare string"')).toEqual({
      auth: {},
      config: "",
    })
  })

  it("非对象 auth 当 {} 处理", () => {
    expect(
      parseCodexConfig(JSON.stringify({ auth: "garbage", config: "x" })),
    ).toEqual({ auth: {}, config: "x" })
    expect(parseCodexConfig(JSON.stringify({ auth: [1, 2] }))).toEqual({
      auth: {},
      config: "",
    })
  })

  it('非字符串 config 当 "" 处理', () => {
    expect(
      parseCodexConfig(
        JSON.stringify({ auth: { OPENAI_API_KEY: "k" }, config: 123 }),
      ),
    ).toEqual({ auth: { OPENAI_API_KEY: "k" }, config: "" })
  })

  it("auth 中非字符串值被过滤", () => {
    expect(
      parseCodexConfig(
        JSON.stringify({
          auth: { OPENAI_API_KEY: "k", bad: 1, also: { x: 1 } },
        }),
      ),
    ).toEqual({ auth: { OPENAI_API_KEY: "k" }, config: "" })
  })
})

describe("codexApiKey / codexConfigToml", () => {
  it("读 API Key 与 TOML 文本", () => {
    const text = JSON.stringify({
      auth: { OPENAI_API_KEY: "sk-abc" },
      config: 'model = "m"\nmodel_provider = "custom"',
    })
    expect(codexApiKey(text)).toBe("sk-abc")
    expect(codexConfigToml(text)).toBe('model = "m"\nmodel_provider = "custom"')
  })

  it("登录态版的 API Key 为空串", () => {
    expect(codexApiKey("{}")).toBe("")
    expect(codexApiKey('{"auth":{}}')).toBe("")
    expect(codexApiKey('{"auth":{"OPENAI_API_KEY":""}}')).toBe("")
  })

  it("空 / 垃圾输入不抛，返回空串", () => {
    expect(codexApiKey("")).toBe("")
    expect(codexApiKey("not-json")).toBe("")
    expect(codexConfigToml("")).toBe("")
    expect(codexConfigToml("not-json")).toBe("")
  })

  it("读预设的 Codex 配置（生产路径）", () => {
    const kimi = CODEX_PROVIDER_PRESETS.find((p) => p.name === "Kimi")!
    // API Key 版：OPENAI_API_KEY 占位为空串（用户填值）。
    expect(codexApiKey(kimi.settingsConfig)).toBe("")
    // config TOML 含 Kimi 端点与模型。
    expect(codexConfigToml(kimi.settingsConfig)).toContain(
      "https://api.moonshot.cn/v1",
    )
    expect(codexConfigToml(kimi.settingsConfig)).toContain("kimi-k2.7-code")
    // OpenAI Official 是登录态版（settingsConfig = "{}"）。
    const official = CODEX_PROVIDER_PRESETS.find(
      (p) => p.name === "OpenAI Official",
    )!
    expect(codexApiKey(official.settingsConfig)).toBe("")
    expect(codexConfigToml(official.settingsConfig)).toBe("")
  })
})

describe("withCodexApiKey", () => {
  it("非空 key 写入 auth.OPENAI_API_KEY，保留 config", () => {
    const text = JSON.stringify({
      auth: {},
      config: 'model = "m"',
    })
    const next = withCodexApiKey(text, "sk-new")
    expect(codexApiKey(next)).toBe("sk-new")
    expect(codexConfigToml(next)).toBe('model = "m"')
  })

  it("空 key 删除 auth.OPENAI_API_KEY，回归登录态版", () => {
    const text = JSON.stringify({
      auth: { OPENAI_API_KEY: "sk-old" },
      config: 'model = "m"',
    })
    const next = withCodexApiKey(text, "")
    expect(codexApiKey(next)).toBe("")
    const parsed = JSON.parse(next) as {
      auth: Record<string, string>
      config: string
    }
    expect(parsed.auth).toEqual({})
    expect(parsed.config).toBe('model = "m"')
  })

  it("保留 auth 其他键", () => {
    const text = JSON.stringify({
      auth: { OPENAI_API_KEY: "sk-old", OTHER_TOKEN: "keep" },
      config: "x",
    })
    const next = withCodexApiKey(text, "sk-new")
    const parsed = JSON.parse(next) as {
      auth: Record<string, string>
    }
    expect(parsed.auth).toEqual({
      OPENAI_API_KEY: "sk-new",
      OTHER_TOKEN: "keep",
    })
  })

  it("往返：写入再删除等于初始的登录态版", () => {
    const start = JSON.stringify({ auth: {}, config: 'model = "m"' })
    const withKey = withCodexApiKey(start, "sk-x")
    const back = withCodexApiKey(withKey, "")
    expect(JSON.parse(back)).toEqual(JSON.parse(start))
  })

  it("空 / 垃圾输入也能工作（按 {} 起步）", () => {
    const next = withCodexApiKey("", "sk-x")
    expect(codexApiKey(next)).toBe("sk-x")
    expect(codexConfigToml(next)).toBe("")
  })
})

describe("withCodexConfigToml", () => {
  it("写入 config 字段，保留 auth", () => {
    const text = JSON.stringify({
      auth: { OPENAI_API_KEY: "sk-x" },
      config: 'model = "old"',
    })
    const next = withCodexConfigToml(text, 'model = "new"')
    expect(codexConfigToml(next)).toBe('model = "new"')
    expect(codexApiKey(next)).toBe("sk-x")
  })

  it("空串作为 config（登录态版的合法形态）", () => {
    const next = withCodexConfigToml('{"auth":{}}', "")
    expect(codexConfigToml(next)).toBe("")
    expect(codexApiKey(next)).toBe("")
  })

  it("保留 auth 的其他键", () => {
    const text = JSON.stringify({
      auth: { OPENAI_API_KEY: "k", OTHER: "keep" },
      config: "x",
    })
    const next = withCodexConfigToml(text, "y")
    const parsed = JSON.parse(next) as {
      auth: Record<string, string>
    }
    expect(parsed.auth).toEqual({ OPENAI_API_KEY: "k", OTHER: "keep" })
  })
})

describe("parseGeminiConfig", () => {
  it("解析 env 与 config（config 以 JSON 文本返回）", () => {
    const text = JSON.stringify({
      env: { GEMINI_API_KEY: "sk", GEMINI_MODEL: "m" },
      config: { model: "m" },
    })
    expect(parseGeminiConfig(text)).toEqual({
      env: { GEMINI_API_KEY: "sk", GEMINI_MODEL: "m" },
      config: JSON.stringify({ model: "m" }),
    })
  })

  it("登录态版（env 为空）", () => {
    expect(parseGeminiConfig("{}")).toEqual({ env: {}, config: "" })
    expect(parseGeminiConfig('{"env":{}}')).toEqual({ env: {}, config: "" })
  })

  it('空 / 垃圾 / 非对象 → {env:{}, config:""}', () => {
    expect(parseGeminiConfig("")).toEqual({ env: {}, config: "" })
    expect(parseGeminiConfig("not-json")).toEqual({ env: {}, config: "" })
    expect(parseGeminiConfig("[1,2]")).toEqual({ env: {}, config: "" })
  })

  it("非对象 env 当 {} 处理", () => {
    expect(parseGeminiConfig(JSON.stringify({ env: "garbage" }))).toEqual({
      env: {},
      config: "",
    })
  })

  it('非对象 config 当 "" 处理（null / 缺失同义）', () => {
    expect(
      parseGeminiConfig(
        JSON.stringify({ env: { GEMINI_API_KEY: "k" }, config: 123 }),
      ),
    ).toEqual({ env: { GEMINI_API_KEY: "k" }, config: "" })
    expect(
      parseGeminiConfig(JSON.stringify({ env: {}, config: null })),
    ).toEqual({ env: {}, config: "" })
  })

  it("env 中非字符串值被过滤", () => {
    expect(
      parseGeminiConfig(
        JSON.stringify({ env: { GEMINI_API_KEY: "k", bad: 1 } }),
      ),
    ).toEqual({ env: { GEMINI_API_KEY: "k" }, config: "" })
  })
})

describe("geminiApiKey / geminiModel / geminiBaseUrl", () => {
  it("读三个 env 字段", () => {
    const text = JSON.stringify({
      env: {
        GEMINI_API_KEY: "sk-x",
        GEMINI_MODEL: "gemini-3.6-pro",
        GOOGLE_GEMINI_BASE_URL: "https://gen.dev",
      },
    })
    expect(geminiApiKey(text)).toBe("sk-x")
    expect(geminiModel(text)).toBe("gemini-3.6-pro")
    expect(geminiBaseUrl(text)).toBe("https://gen.dev")
  })

  it("空 / 垃圾输入不抛，返回空串", () => {
    expect(geminiApiKey("")).toBe("")
    expect(geminiModel("not-json")).toBe("")
    expect(geminiBaseUrl("")).toBe("")
  })

  it("读预设的 Gemini 配置（生产路径）", () => {
    const oauth = GEMINI_PROVIDER_PRESETS.find(
      (p) => p.name === "Google Gemini",
    )!
    expect(geminiApiKey(oauth.settingsConfig)).toBe("")
    expect(geminiModel(oauth.settingsConfig)).toBe("")

    const apiKey = GEMINI_PROVIDER_PRESETS.find(
      (p) => p.name === "Google Gemini (API Key)",
    )!
    expect(geminiApiKey(apiKey.settingsConfig)).toBe("")
    expect(geminiModel(apiKey.settingsConfig)).toBe("gemini-3.6-pro")
    expect(geminiBaseUrl(apiKey.settingsConfig)).toBe(
      "https://generativelanguage.googleapis.com",
    )

    const eflow = GEMINI_PROVIDER_PRESETS.find((p) => p.name === "E-FlowCode")!
    expect(geminiApiKey(eflow.settingsConfig)).toBe("")
    expect(geminiModel(eflow.settingsConfig)).toBe("gemini-3.6-flash")
    expect(geminiBaseUrl(eflow.settingsConfig)).toBe("https://e-flowcode.cc")
    // E-FlowCode 的 config 是个对象，parseGeminiConfig 以 JSON 文本返回。
    const configJson = parseGeminiConfig(eflow.settingsConfig).config
    expect(configJson).toContain("previewFeatures")
    expect(configJson).toContain("sessionRetention")
  })
})

describe("withGeminiEnv", () => {
  it("合并 patch 进 env，保留 config", () => {
    const text = JSON.stringify({
      env: { GEMINI_MODEL: "old" },
      config: { general: { x: 1 } },
    })
    const next = withGeminiEnv(text, {
      GEMINI_API_KEY: "sk-new",
      GEMINI_MODEL: "new",
    })
    expect(geminiApiKey(next)).toBe("sk-new")
    expect(geminiModel(next)).toBe("new")
    // config 原样保留。
    const parsed = JSON.parse(next) as { config?: unknown }
    expect(parsed.config).toEqual({ general: { x: 1 } })
  })

  it("空串值删除 env 键（GEMINI_API_KEY 删除即回归登录态版）", () => {
    const text = JSON.stringify({
      env: { GEMINI_API_KEY: "sk", GEMINI_MODEL: "m" },
    })
    const next = withGeminiEnv(text, { GEMINI_API_KEY: "" })
    expect(geminiApiKey(next)).toBe("")
    const parsed = JSON.parse(next) as { env: Record<string, string> }
    expect(parsed.env).toEqual({ GEMINI_MODEL: "m" })
  })

  it("往返：写入再删除等于初始", () => {
    const start = JSON.stringify({ env: { GEMINI_MODEL: "m" } })
    const withKey = withGeminiEnv(start, { GEMINI_API_KEY: "sk-x" })
    const back = withGeminiEnv(withKey, { GEMINI_API_KEY: "" })
    expect(JSON.parse(back)).toEqual(JSON.parse(start))
  })

  it("空 / 垃圾输入也能工作（按 {env:{}} 起步）", () => {
    const next = withGeminiEnv("", { GEMINI_API_KEY: "sk-x" })
    expect(geminiApiKey(next)).toBe("sk-x")
  })
})

describe("withGeminiConfigJson", () => {
  it("写入合法 JSON 到 config 字段，保留 env", () => {
    const text = JSON.stringify({ env: { GEMINI_API_KEY: "k" } })
    const next = withGeminiConfigJson(text, '{"general":{"x":1}}')
    const parsed = JSON.parse(next) as {
      env: Record<string, string>
      config: { general: { x: number } }
    }
    expect(parsed.env).toEqual({ GEMINI_API_KEY: "k" })
    expect(parsed.config).toEqual({ general: { x: 1 } })
  })

  it("空串删除 config 键", () => {
    const text = JSON.stringify({
      env: { GEMINI_API_KEY: "k" },
      config: { general: { x: 1 } },
    })
    const next = withGeminiConfigJson(text, "")
    const parsed = JSON.parse(next) as {
      env: Record<string, string>
      config?: unknown
    }
    expect(parsed.env).toEqual({ GEMINI_API_KEY: "k" })
    expect(parsed.config).toBeUndefined()
  })

  it("非法 JSON 原样不动返回原 text", () => {
    const text = JSON.stringify({ env: { GEMINI_API_KEY: "k" } })
    expect(withGeminiConfigJson(text, "{not-json")).toBe(text)
  })

  it("非对象 JSON（数组 / null / 字符串）原样不动", () => {
    const text = JSON.stringify({ env: {} })
    expect(withGeminiConfigJson(text, "[1,2]")).toBe(text)
    expect(withGeminiConfigJson(text, "null")).toBe(text)
    expect(withGeminiConfigJson(text, '"str"')).toBe(text)
    expect(withGeminiConfigJson(text, "123")).toBe(text)
  })

  it("替换已有 config", () => {
    const text = JSON.stringify({
      env: {},
      config: { old: 1 },
    })
    const next = withGeminiConfigJson(text, '{"new":2}')
    const parsed = JSON.parse(next) as { config: { new: number } }
    expect(parsed.config).toEqual({ new: 2 })
  })
})

describe("parseGrokConfig", () => {
  it("从 settingsConfig 提取 config TOML 文本", () => {
    const text = JSON.stringify({
      config: '[model.cc-one]\nmodel = "grok-4.5"',
    })
    expect(parseGrokConfig(text)).toEqual({
      config: '[model.cc-one]\nmodel = "grok-4.5"',
    })
  })

  it("空 / 缺 config 字段 → 空目标", () => {
    expect(parseGrokConfig("{}")).toEqual({ config: "" })
    expect(parseGrokConfig(JSON.stringify({ config: "" }))).toEqual({
      config: "",
    })
  })

  it("垃圾输入宽容归一（表单遇手改坏的快照不崩）", () => {
    expect(parseGrokConfig("")).toEqual({ config: "" })
    expect(parseGrokConfig("not-json")).toEqual({ config: "" })
    expect(parseGrokConfig("[1,2]")).toEqual({ config: "" })
    expect(parseGrokConfig('"a bare string"')).toEqual({ config: "" })
    // 非字符串 config（数字 / 对象）当 "" 处理。
    expect(parseGrokConfig(JSON.stringify({ config: 123 }))).toEqual({
      config: "",
    })
    expect(parseGrokConfig(JSON.stringify({ config: { a: 1 } }))).toEqual({
      config: "",
    })
  })
})

describe("grokConfigToml / withGrokConfigToml", () => {
  it("grokConfigToml 读出 config TOML 文本", () => {
    const text = JSON.stringify({ config: 'model = "grok-4.5"' })
    expect(grokConfigToml(text)).toBe('model = "grok-4.5"')
  })

  it("withGrokConfigToml 把 TOML 写回 config 字段", () => {
    const text = JSON.stringify({ config: "old" })
    const next = withGrokConfigToml(text, '[model.cc-one]\nmodel = "grok-4.5"')
    expect(grokConfigToml(next)).toBe('[model.cc-one]\nmodel = "grok-4.5"')
  })

  it("withGrokConfigToml 对坏输入也产出合法 settingsConfig", () => {
    const next = withGrokConfigToml("not-json", "new-toml")
    expect(JSON.parse(next)).toEqual({ config: "new-toml" })
  })

  it("Grok 预设的 settingsConfig 经往返读写不丢格式", () => {
    for (const preset of GROK_PROVIDER_PRESETS) {
      const toml = grokConfigToml(preset.settingsConfig)
      const back = withGrokConfigToml("{}", toml)
      expect(grokConfigToml(back)).toBe(toml)
    }
  })
})

describe("parseOpenCodeConfig", () => {
  it("从 settingsConfig 提取 npm / options / models（顶层 name 不进结构化视图）", () => {
    const text = JSON.stringify({
      npm: "@ai-sdk/openai-compatible",
      options: {
        baseURL: "https://api.deepseek.com",
        apiKey: "sk-xxx",
        headers: { "Helicone-Auth": "h-xxx" },
      },
      models: { "deepseek-chat": { name: "DeepSeek Chat" } },
      name: "DeepSeek",
    })
    expect(parseOpenCodeConfig(text)).toEqual({
      npm: "@ai-sdk/openai-compatible",
      baseURL: "https://api.deepseek.com",
      apiKey: "sk-xxx",
      headers: { "Helicone-Auth": "h-xxx" },
      models: { "deepseek-chat": { name: "DeepSeek Chat" } },
    })
  })

  it("空 / 缺字段 → 各字段归零", () => {
    const zero = {
      npm: "",
      baseURL: "",
      apiKey: "",
      headers: {},
      models: {},
    }
    expect(parseOpenCodeConfig("{}")).toEqual(zero)
    expect(
      parseOpenCodeConfig(JSON.stringify({ npm: "@ai-sdk/openai" })),
    ).toEqual({ ...zero, npm: "@ai-sdk/openai" })
  })

  it("垃圾输入宽容归一（表单遇手改坏的快照不崩）", () => {
    const zero = {
      npm: "",
      baseURL: "",
      apiKey: "",
      headers: {},
      models: {},
    }
    expect(parseOpenCodeConfig("")).toEqual(zero)
    expect(parseOpenCodeConfig("not-json")).toEqual(zero)
    expect(parseOpenCodeConfig("[1,2]")).toEqual(zero)
    expect(parseOpenCodeConfig('"a bare string"')).toEqual(zero)
  })

  it("非对象 options / models / headers 按空处理", () => {
    const cfg = parseOpenCodeConfig(
      JSON.stringify({ options: "garbage", models: [1, 2], headers: 123 }),
    )
    expect(cfg.baseURL).toBe("")
    expect(cfg.headers).toEqual({})
    expect(cfg.models).toEqual({})
  })

  it("headers 只保留字符串值（数字 / null 丢弃）", () => {
    const cfg = parseOpenCodeConfig(
      JSON.stringify({
        options: { headers: { "X-A": "1", "X-B": 2, "X-C": null } },
      }),
    )
    expect(cfg.headers).toEqual({ "X-A": "1" })
  })

  it("models 子条目无 name（或 name 非字符串）→ 空对象", () => {
    const cfg = parseOpenCodeConfig(
      JSON.stringify({
        models: { m1: { contextWindow: 8192 }, m2: { name: 9 } },
      }),
    )
    expect(cfg.models).toEqual({ m1: {}, m2: {} })
  })
})

describe("openCode* 读取器", () => {
  const text = JSON.stringify({
    npm: "@ai-sdk/anthropic",
    options: {
      baseURL: "https://x",
      apiKey: "k",
      headers: { Authorization: "Bearer t" },
    },
    models: { m1: { name: "M1" } },
  })

  it("各读取器分别取 npm / baseURL / apiKey / headers / models", () => {
    expect(openCodeNpm(text)).toBe("@ai-sdk/anthropic")
    expect(openCodeBaseUrl(text)).toBe("https://x")
    expect(openCodeApiKey(text)).toBe("k")
    expect(openCodeHeaders(text)).toEqual({ Authorization: "Bearer t" })
    expect(openCodeModels(text)).toEqual({ m1: { name: "M1" } })
  })
})

describe("withOpenCode* 写入器", () => {
  it("withOpenCodeNpm 写 npm，保留 options / models / 顶层键（如 name）", () => {
    const text = JSON.stringify({
      npm: "old",
      options: { baseURL: "https://x" },
      models: { m1: { name: "M1" } },
      name: "DeepSeek",
    })
    const next = withOpenCodeNpm(text, "@ai-sdk/openai-compatible")
    expect(openCodeNpm(next)).toBe("@ai-sdk/openai-compatible")
    expect(openCodeBaseUrl(next)).toBe("https://x")
    expect(openCodeModels(next)).toEqual({ m1: { name: "M1" } })
    expect(JSON.parse(next).name).toBe("DeepSeek")
  })

  it("withOpenCodeBaseUrl / withOpenCodeApiKey 写 options，保留 options 其它键", () => {
    const text = JSON.stringify({ options: { baseURL: "old", extra: "keep" } })
    const next = withOpenCodeBaseUrl(
      withOpenCodeApiKey(text, "k"),
      "https://new",
    )
    expect(openCodeBaseUrl(next)).toBe("https://new")
    expect(openCodeApiKey(next)).toBe("k")
    expect(JSON.parse(next).options.extra).toBe("keep")
  })

  it("空 baseURL / apiKey → 删键（回归无值版）", () => {
    const text = JSON.stringify({ options: { baseURL: "x", apiKey: "k" } })
    const next = withOpenCodeBaseUrl(withOpenCodeApiKey(text, ""), "")
    expect(JSON.parse(next).options).toEqual({})
  })

  it("withOpenCodeHeaders 整块替换；空对象 → 删 options.headers 键", () => {
    const text = JSON.stringify({ options: { headers: { old: "x" } } })
    const set = withOpenCodeHeaders(text, { Authorization: "Bearer t" })
    expect(openCodeHeaders(set)).toEqual({ Authorization: "Bearer t" })
    const cleared = withOpenCodeHeaders(set, {})
    expect(JSON.parse(cleared).options.headers).toBeUndefined()
  })

  it("withOpenCodeModels 整块替换；空 → 删 models 键；空白 id 丢弃", () => {
    const text = JSON.stringify({ models: { old: { name: "O" } }, npm: "n" })
    const set = withOpenCodeModels(text, {
      "new-model": { name: "New" },
      "   ": { name: "blank-id" },
    })
    expect(openCodeModels(set)).toEqual({ "new-model": { name: "New" } })
    expect(JSON.parse(set).npm).toBe("n")
    const cleared = withOpenCodeModels(set, {})
    expect(JSON.parse(cleared).models).toBeUndefined()
  })

  it("写入器对坏输入也产出合法 settingsConfig（不崩）", () => {
    expect(JSON.parse(withOpenCodeNpm("not-json", "@ai-sdk/openai"))).toEqual({
      npm: "@ai-sdk/openai",
    })
    expect(JSON.parse(withOpenCodeBaseUrl("not-json", "https://x"))).toEqual({
      options: { baseURL: "https://x" },
    })
    expect(JSON.parse(withOpenCodeApiKey("not-json", "k"))).toEqual({
      options: { apiKey: "k" },
    })
  })

  it("models 经写入往返（仅 model_id + name）", () => {
    const start = "{}"
    const next = withOpenCodeModels(start, {
      "deepseek-chat": { name: "DeepSeek Chat" },
      "deepseek-reasoner": { name: "DeepSeek Reasoner" },
    })
    expect(openCodeModels(next)).toEqual({
      "deepseek-chat": { name: "DeepSeek Chat" },
      "deepseek-reasoner": { name: "DeepSeek Reasoner" },
    })
  })
})
