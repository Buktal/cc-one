import { describe, expect, it } from "vitest"
import { PROVIDER_PRESETS } from "@/features/providers/presets"
import type { Provider } from "@/types/generated/bindings"
import { emptyProvider, providerFromPreset } from "../derive"
import {
  authFieldKey,
  configApiKey,
  configAuthField,
  configEndpoint,
  configRoleFields,
  configRoleHasOneM,
  configRoleModel,
  configRoleName,
  hasOneM,
  normalizeBasicFieldsInText,
  parseSettingsConfig,
  providerApiKey,
  providerEndpoint,
  providerModel,
  setModelOneM,
  stripOneM,
  switchAuthField,
  withAllRolesFromFirstInText,
  withAllRolesInText,
  withBasicFields,
  withBasicFieldsInText,
  withRoleModelInText,
  withRoleNameInText,
  withRoleOneMInText,
} from "./claude"

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

describe("parseSettingsConfig", () => {
  it("garbage, empty or non-object top level → {} (corrupt snapshots don't throw)", () => {
    expect(parseSettingsConfig("")).toEqual({})
    expect(parseSettingsConfig("not-json")).toEqual({})
    expect(parseSettingsConfig("[1,2]")).toEqual({})
    expect(parseSettingsConfig('"a bare string"')).toEqual({})
  })

  it("a non-object env (string, array) is dropped to {}", () => {
    expect(parseSettingsConfig(JSON.stringify({ env: "garbage" }))).toEqual({
      env: {},
    })
    expect(parseSettingsConfig(JSON.stringify({ env: [1, 2] }))).toEqual({
      env: {},
    })
  })
})

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
  it("inherits the preset's category and copies the preset snapshot verbatim", () => {
    const kimi = PROVIDER_PRESETS.find((p) => p.name === "Kimi")
    expect(kimi).toBeDefined()
    const draft = providerFromPreset(kimi!)
    // 草稿继承预设分类（cn_official 保持国内官方），id 留空让 save 分配。
    expect(draft.id).toBe("")
    expect(draft.category).toBe("cn_official")
    expect(draft.name).toBe("Kimi")
    expect(draft.settingsConfig).toBe(kimi!.settingsConfig)
    // derive 读函数直接回填表单字段，无需另起一套解析。
    expect(providerEndpoint(draft)).toBe("https://api.moonshot.cn/anthropic")
    expect(providerModel(draft)).toBe("kimi-k2.7-code")
    expect(providerApiKey(draft)).toBe("")
  })

  it("keeps a cloud_provider preset's category（列表分类与切换检查按它区分）", () => {
    const bedrock = PROVIDER_PRESETS.find(
      (p) => p.name === "AWS Bedrock (AKSK)",
    )
    expect(bedrock).toBeDefined()
    const draft = providerFromPreset(bedrock!)
    expect(draft.category).toBe("cloud_provider")
    expect(draft.settingsConfig).toBe(bedrock!.settingsConfig)
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

  it("does not propagate the picked model's 1M marker — each role keeps its own 1M state", () => {
    // 源模型带 [1M]：opus 自己勾着 → 保留标记；其余角色原本无标记 → 裸名。
    const next = withAllRolesFromFirstInText(
      configWith({ ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.1[1M]" }),
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("glm-5.1[1M]")
    // fable 无自有值、经回填链（→ OPUS）读到 [1M]，视为已勾 1M 保留标记。
    expect(env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_SUBAGENT_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-5.1")
    // 显示名一律不带标记。
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME).toBe("glm-5.1")
  })

  it("keeps a role's own 1M marker when the primary model is marker-free（一键设置不清掉已勾的 1M）", () => {
    // 用户 bug 场景：主模型无标记 + sonnet 已勾 1M → 一键设置统一模型名，
    // sonnet 的 [1M] 保留。
    const next = withAllRolesFromFirstInText(
      configWith({
        ANTHROPIC_MODEL: "glm-5.1",
        ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1[1M]",
      }),
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("glm-5.1[1M]")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("glm-5.1")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("glm-5.1")
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
  it("writes the bare model to every role — the input's marker never propagates", () => {
    // 传入模型带 [1M] 但目标角色原本无勾选 → 全部写裸名（标记归各角色
    // checkbox 自持，不随传播值走）。Haiku 本来就不带标记。
    const next = withAllRolesInText(configWith({}), "my-model[1M]")
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_SUBAGENT_MODEL).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBe("my-model")
    // 显示名一律不带标记。
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME).toBe("my-model")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME).toBe("my-model")
  })

  it("keeps a role's own 1M marker while unify the model name", () => {
    const next = withAllRolesInText(
      configWith({ ANTHROPIC_DEFAULT_SONNET_MODEL: "old[1M]" }),
      "new-model",
    )
    const env = envOf(next)
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("new-model[1M]")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("new-model")
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

describe("normalizeBasicFieldsInText", () => {
  it("trims trailing spaces from the endpoint at save", () => {
    const next = normalizeBasicFieldsInText(
      JSON.stringify({ env: { ANTHROPIC_BASE_URL: "https://x.dev  " } }),
    )
    expect(configEndpoint(next)).toBe("https://x.dev")
  })

  it("preserves everything the form does not own", () => {
    const next = normalizeBasicFieldsInText(
      JSON.stringify({
        includeCoAuthoredBy: false,
        env: {
          ANTHROPIC_BASE_URL: "  https://x.dev ",
          ANTHROPIC_AUTH_TOKEN: "sk",
          ANTHROPIC_MODEL: "keep-me",
          ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "我的主力",
        },
      }),
    )
    expect(JSON.parse(next)).toEqual({
      includeCoAuthoredBy: false,
      env: {
        ANTHROPIC_BASE_URL: "https://x.dev",
        ANTHROPIC_AUTH_TOKEN: "sk",
        ANTHROPIC_MODEL: "keep-me",
        ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: "我的主力",
      },
    })
  })

  it("removes an empty endpoint / key at save", () => {
    const next = normalizeBasicFieldsInText(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: "  ",
          ANTHROPIC_AUTH_TOKEN: "",
          ANTHROPIC_MODEL: "keep-me",
        },
      }),
    )
    expect(JSON.parse(next)).toEqual({ env: { ANTHROPIC_MODEL: "keep-me" } })
  })

  it("keeps the key under the selected auth spelling only", () => {
    const next = normalizeBasicFieldsInText(
      JSON.stringify({ env: { ANTHROPIC_API_KEY: "sk-legacy" } }),
    )
    expect(configAuthField(next)).toBe("api_key")
    expect(configApiKey(next)).toBe("sk-legacy")
  })

  it("normalizes a materialized snapshot — no placeholder sneaks back through the endpoint", () => {
    // 旧镜像态把物化前的占位符端点写回快照（物化不完整，后端切换时会拦截
    // 未物化占位符）；归一重新读同一文本，物化后的端点原样保留。
    const next = normalizeBasicFieldsInText(
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL:
            "https://bedrock-runtime.ap-northeast-1.amazonaws.com",
          ANTHROPIC_AUTH_TOKEN: "sk",
        },
      }),
    )
    expect(configEndpoint(next)).toBe(
      "https://bedrock-runtime.ap-northeast-1.amazonaws.com",
    )
  })
})

describe("form round-trip: sibling fields derive from prev（镜像态消除后的生产路径）", () => {
  // 无镜像态下组件的写回形态：改一个字段时，其余字段从 prev 派生读回——与旧
  // 镜像态同值（镜像与文本解析态等价时）。这些断言钉住生产路径的确切表达式，
  // 与「编辑不吞半截 JSON」的 guardedRewrite 一起构成写回的测试面。

  it("an endpoint edit preserves the key value and the selected auth spelling", () => {
    const text = JSON.stringify({
      env: { ANTHROPIC_BASE_URL: "old", ANTHROPIC_API_KEY: "sk" },
    })
    const next = withBasicFieldsInText(text, {
      endpoint: "new",
      apiKey: configApiKey(text),
      authField: configAuthField(text),
    })
    expect(JSON.parse(next)).toEqual({
      env: { ANTHROPIC_BASE_URL: "new", ANTHROPIC_API_KEY: "sk" },
    })
    expect(configAuthField(next)).toBe("api_key")
  })

  it("an apiKey edit preserves the endpoint and the auth spelling", () => {
    const text = JSON.stringify({
      env: {
        ANTHROPIC_BASE_URL: "https://x.dev",
        ANTHROPIC_AUTH_TOKEN: "old",
      },
    })
    const next = withBasicFieldsInText(text, {
      endpoint: configEndpoint(text),
      apiKey: "new",
      authField: configAuthField(text),
    })
    expect(configEndpoint(next)).toBe("https://x.dev")
    expect(configApiKey(next)).toBe("new")
    expect(configAuthField(next)).toBe("auth_token")
  })

  it("an auth-field toggle derives the source spelling from prev", () => {
    const text = JSON.stringify({ env: { ANTHROPIC_AUTH_TOKEN: "sk" } })
    const next = switchAuthField(text, configAuthField(text), "api_key")
    expect(JSON.parse(next)).toEqual({ env: { ANTHROPIC_API_KEY: "sk" } })
    expect(configAuthField(next)).toBe("api_key")
  })
})
