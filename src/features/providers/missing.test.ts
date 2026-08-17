import { describe, expect, it } from "vitest"
import type { Provider } from "@/types/generated/bindings"
import { emptyProvider } from "./derive"
import { providerMissingRequired } from "./missing"

/** A provider whose settingsConfig carries a full env block. */
function provider(config: string): Provider {
  return {
    ...emptyProvider(),
    settingsConfig: config,
  }
}

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

  it("checks each app's own required keys (codex/gemini/grok/opencode)", () => {
    // 各 app 构造「缺必填项」的供应商：确认框均要触发（#61——原实现只读
    // ANTHROPIC_* 键，其余四 app 恒为空数组）。
    // codex：第三方预设形状，auth 占位空 key + TOML base_url 已填 → 缺 key。
    const codex = {
      ...p(
        JSON.stringify({
          auth: { OPENAI_API_KEY: "" },
          config:
            'model_provider = "kimi"\n[model_providers.kimi]\nbase_url = "https://api.moonshot.cn/v1"\n',
        }),
        "cn_official" as const,
      ),
      app: "codex" as const,
    }
    expect(providerMissingRequired(codex)).toEqual(["apiKey"])
    // codex 登录态版（无 auth key、无端点）→ 不告警。
    const codexLogin = {
      ...p("{}", "official" as const),
      app: "codex" as const,
    }
    expect(providerMissingRequired(codexLogin)).toEqual([])
    // codex 官方 API Key 版：空占位 → 告警（official 分类也不例外）。
    const codexOfficialKey = {
      ...p('{"auth":{"OPENAI_API_KEY":""}}', "official" as const),
      app: "codex" as const,
    }
    expect(providerMissingRequired(codexOfficialKey)).toEqual(["apiKey"])

    // gemini：官方 API Key 版预设（GEMINI_API_KEY 空占位）→ 缺 key。
    const gemini = {
      ...p(
        JSON.stringify({
          env: {
            GEMINI_API_KEY: "",
            GOOGLE_GEMINI_BASE_URL: "https://generativelanguage.googleapis.com",
          },
        }),
        "official" as const,
      ),
      app: "gemini" as const,
    }
    expect(providerMissingRequired(gemini)).toEqual(["apiKey"])
    // gemini OAuth 版（无 key 无端点）→ 不告警。
    const geminiOauth = {
      ...p("{}", "official" as const),
      app: "gemini" as const,
    }
    expect(providerMissingRequired(geminiOauth)).toEqual([])
    // gemini 第三方聚合无 key → 缺 key + 缺端点（键不存在按缺失算）。
    const geminiAgg = {
      ...p(
        JSON.stringify({ env: { GEMINI_MODEL: "m" } }),
        "aggregator" as const,
      ),
      app: "gemini" as const,
    }
    expect(providerMissingRequired(geminiAgg)).toEqual(["endpoint", "apiKey"])

    // grok：第三方预设形状（api_key 空占位）→ 缺 key；端点已填不缺。
    const grok = {
      ...p(
        JSON.stringify({
          config:
            '[model.cc-one]\nmodel = "grok-4.5"\nbase_url = "https://api.x.ai/v1"\napi_key = ""\n',
        }),
        "aggregator" as const,
      ),
      app: "grok" as const,
    }
    expect(providerMissingRequired(grok)).toEqual(["apiKey"])
    // grok 官方登录态（空 config）→ 不告警。
    const grokLogin = {
      ...p("{}", "official" as const),
      app: "grok" as const,
    }
    expect(providerMissingRequired(grokLogin)).toEqual([])

    // opencode：附加模式 key 一律必填（预设 apiKey 空占位）→ 缺 key。
    const opencode = {
      ...p(
        JSON.stringify({
          npm: "@ai-sdk/openai",
          options: { baseURL: "https://api.openai.com/v1", apiKey: "" },
        }),
        "official" as const,
      ),
      app: "opencode" as const,
    }
    expect(providerMissingRequired(opencode)).toEqual(["apiKey"])
    // opencode baseURL 空占位（非缺省）→ 缺端点；缺省 baseURL（走 npm SDK
    // 自带端点）不告警。
    const opencodeNoEndpoint = {
      ...p(
        JSON.stringify({
          npm: "@ai-sdk/openai",
          options: { apiKey: "sk-x", baseURL: "" },
        }),
        "official" as const,
      ),
      app: "opencode" as const,
    }
    expect(providerMissingRequired(opencodeNoEndpoint)).toEqual(["endpoint"])
    const opencodeDefaultEndpoint = {
      ...p(
        JSON.stringify({
          npm: "@ai-sdk/openai",
          options: { apiKey: "sk-x" },
        }),
        "official" as const,
      ),
      app: "opencode" as const,
    }
    expect(providerMissingRequired(opencodeDefaultEndpoint)).toEqual([])
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
