import { describe, expect, it } from "vitest"
import { CODEX_PROVIDER_PRESETS } from "@/features/providers/codex-presets"
import {
  codexApiKey,
  codexConfigToml,
  emptyProvider,
  extractTemplateVars,
  filterProviders,
  geminiBaseUrl,
  geminiModel,
  metaTemplateValues,
  providerFromPreset,
  providerLiveKey,
  providerLiveManaged,
  replaceTemplateVarsInText,
  restoreTemplatePlaceholders,
  withMetaTemplateValues,
} from "@/features/providers/derive"
import { GEMINI_PROVIDER_PRESETS } from "@/features/providers/gemini-presets"
import { PROVIDER_PRESETS } from "@/features/providers/presets"
import type { Provider } from "@/types/generated/bindings"

/** A provider whose settingsConfig carries a full env block. */
function provider(config: string): Provider {
  return {
    ...emptyProvider(),
    settingsConfig: config,
  }
}

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
    // 草稿继承预设分类（Kimi 预设是 cn_official，不再抹成 custom）。
    expect(draft.category).toBe("cn_official")
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
