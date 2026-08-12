// 预设清单的守门测试：18 个 Claude 内置预设必须与需求清单一字不差（数量、
// 名称、顺序、category 映射），排除项（OAuth 类 / gemini_native / openai_chat
// 格式）一个都不许泄漏进清单，每项 settingsConfig 都是合法 JSON 且含 env 块。
// 同时守住 Codex 17 / Gemini 6 两个精选清单的数量，以及 presetsForApp 的应用
// 维度分派。清单文件是单一事实来源——这里把「需求」固化成语义断言，任何
// 增删改都会先在这里红掉。

import { describe, expect, it } from "vitest"
import { CODEX_PROVIDER_PRESETS } from "@/features/providers/codex-presets"
import {
  parseOpenCodeConfig,
  providerEndpoint,
  providerFromPreset,
} from "@/features/providers/derive"
import { GEMINI_PROVIDER_PRESETS } from "@/features/providers/gemini-presets"
import { GROK_PROVIDER_PRESETS } from "@/features/providers/grok-presets"
import { OPENCODE_PROVIDER_PRESETS } from "@/features/providers/opencode-presets"
import { PROVIDER_PRESETS, presetsForApp } from "@/features/providers/presets"

import type { App, ProviderCategory } from "@/types/generated/bindings"

/** 权威清单：名称与顺序不得增删改（官方/云 3 + 国内大厂 11 + 热门聚合 4）。 */
const EXPECTED_NAMES = [
  "Claude Official",
  "AWS Bedrock (AKSK)",
  "AWS Bedrock (API Key)",
  "Kimi",
  "Kimi For Coding",
  "DeepSeek",
  "Zhipu GLM",
  "火山 Agentplan",
  "DouBaoSeed",
  "百度千帆",
  "阿里百炼 For Coding",
  "StepFun",
  "MiniMax",
  "小米 MiMo",
  "SiliconFlow",
  "OpenRouter",
  "ModelScope",
  "Novita AI",
]

/** 每个预设的 category 归属（名称 → 分类）。 */
const NAME_CATEGORY: Record<string, ProviderCategory> = {
  "Claude Official": "official",
  "AWS Bedrock (AKSK)": "cloud_provider",
  "AWS Bedrock (API Key)": "cloud_provider",
  Kimi: "cn_official",
  "Kimi For Coding": "cn_official",
  DeepSeek: "cn_official",
  "Zhipu GLM": "cn_official",
  "火山 Agentplan": "cn_official",
  DouBaoSeed: "cn_official",
  百度千帆: "cn_official",
  "阿里百炼 For Coding": "cn_official",
  StepFun: "cn_official",
  MiniMax: "cn_official",
  "小米 MiMo": "cn_official",
  SiliconFlow: "aggregator",
  OpenRouter: "aggregator",
  ModelScope: "aggregator",
  "Novita AI": "aggregator",
}

/** 排除项黑名单（正则）：OAuth 类（GitHub Copilot / Codex / xAI）与 gemini_native /
 *  openai_chat 格式，断言其不出现在任何预设的名称或 settingsConfig 里。`xai` 用
 *  词边界匹配——SiliconFlow 的模型名 MiniMaxAI 合法含 "xai" 子串，只有独立的
 *  xAI 供应商名才算泄漏。 */
const BLACKLIST_PATTERNS = [
  /oauth/,
  /github/,
  /copilot/,
  /codex/,
  /grok/,
  /\bxai\b/,
  /gemini/,
  /openai/,
  /generativelanguage/,
  /gpt-5/,
]

describe("PROVIDER_PRESETS", () => {
  it("总数 18 且名称与顺序与需求清单完全一致", () => {
    expect(PROVIDER_PRESETS).toHaveLength(18)
    expect(PROVIDER_PRESETS.map((p) => p.name)).toEqual(EXPECTED_NAMES)
  })

  it("名称唯一", () => {
    const names = PROVIDER_PRESETS.map((p) => p.name)
    expect(new Set(names).size).toBe(names.length)
  })

  it("category 映射正确且分组合计为 1/2/11/4", () => {
    const counts: Record<ProviderCategory, number> = {
      official: 0,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 0,
      custom: 0,
    }
    for (const preset of PROVIDER_PRESETS) {
      expect(preset.category).toBe(NAME_CATEGORY[preset.name])
      counts[preset.category] += 1
    }
    expect(counts).toEqual({
      official: 1,
      cloud_provider: 2,
      cn_official: 11,
      aggregator: 4,
      custom: 0,
    })
  })

  it("无排除项泄漏（OAuth / gemini_native / openai_chat）", () => {
    const text = PROVIDER_PRESETS.map((p) => `${p.name} ${p.settingsConfig}`)
      .join("\n")
      .toLowerCase()
    for (const pattern of BLACKLIST_PATTERNS) {
      expect(text).not.toMatch(pattern)
    }
  })

  it("每项 settingsConfig 是合法 JSON 且含 env 对象", () => {
    for (const preset of PROVIDER_PRESETS) {
      const parsed: unknown = JSON.parse(preset.settingsConfig)
      expect(
        parsed !== null && typeof parsed === "object" && !Array.isArray(parsed),
      ).toBe(true)
      const env = (parsed as { env?: unknown }).env
      expect(
        env !== null && typeof env === "object" && !Array.isArray(env),
      ).toBe(true)
    }
  })

  it("每个预设都带分类的必填元数据（websiteUrl / icon / iconColor）", () => {
    for (const preset of PROVIDER_PRESETS) {
      expect(preset.websiteUrl).toBeTruthy()
      expect(preset.icon).toBeTruthy()
      expect(preset.iconColor).toBeTruthy()
    }
  })

  it("预设可被表单读取：providerFromPreset 回填端点与模型", () => {
    const openrouter = PROVIDER_PRESETS.find((p) => p.name === "OpenRouter")
    expect(openrouter).toBeDefined()
    const draft = providerFromPreset(openrouter!)
    expect(providerEndpoint(draft)).toBe("https://openrouter.ai/api")
  })
})

/** Codex 权威清单：官方 2 + 国内大厂 11 + 热门聚合 4 = 17。 */
const EXPECTED_CODEX_NAMES = [
  "OpenAI Official",
  "OpenAI",
  "Kimi",
  "Kimi For Coding",
  "DeepSeek",
  "Zhipu GLM",
  "火山 Agentplan",
  "DouBaoSeed",
  "百度千帆",
  "阿里百炼",
  "StepFun",
  "MiniMax",
  "小米 MiMo",
  "SiliconFlow",
  "OpenRouter",
  "ModelScope",
  "Novita AI",
]

/** Gemini 权威清单：官方 2 + 热门聚合 4 = 6。 */
const EXPECTED_GEMINI_NAMES = [
  "Google Gemini",
  "Google Gemini (API Key)",
  "OpenRouter",
  "TheRouter",
  "千象 Qiniu",
  "E-FlowCode",
]

/** Grok 权威清单：官方 2 + 热门聚合 4 = 6（无国内大厂 grok 兼容端点）。 */
const EXPECTED_GROK_NAMES = [
  "Grok Official",
  "xAI (Grok)",
  "OpenRouter",
  "TheRouter",
  "千象 Qiniu",
  "E-FlowCode",
]

describe("CODEX_PROVIDER_PRESETS", () => {
  it("总数 17 且名称与顺序与需求清单完全一致", () => {
    expect(CODEX_PROVIDER_PRESETS).toHaveLength(17)
    expect(CODEX_PROVIDER_PRESETS.map((p) => p.name)).toEqual(
      EXPECTED_CODEX_NAMES,
    )
  })

  it("名称唯一", () => {
    const names = CODEX_PROVIDER_PRESETS.map((p) => p.name)
    expect(new Set(names).size).toBe(names.length)
  })

  it("category 分组合计为 官方 2 / 国内 11 / 聚合 4", () => {
    const counts: Record<ProviderCategory, number> = {
      official: 0,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 0,
      custom: 0,
    }
    for (const preset of CODEX_PROVIDER_PRESETS) {
      counts[preset.category] += 1
    }
    expect(counts).toEqual({
      official: 2,
      cloud_provider: 0,
      cn_official: 11,
      aggregator: 4,
      custom: 0,
    })
  })

  it("每项 settingsConfig 是合法 JSON 对象（auth + config 结构或空对象）", () => {
    for (const preset of CODEX_PROVIDER_PRESETS) {
      const parsed: unknown = JSON.parse(preset.settingsConfig)
      expect(
        parsed !== null && typeof parsed === "object" && !Array.isArray(parsed),
      ).toBe(true)
    }
  })

  it('OpenAI Official 是登录态版（settingsConfig 为 "{}"，无 OPENAI_API_KEY）', () => {
    const official = CODEX_PROVIDER_PRESETS.find(
      (p) => p.name === "OpenAI Official",
    )
    expect(official).toBeDefined()
    expect(official!.settingsConfig).toBe("{}")
  })

  it("每个预设都带必填元数据（websiteUrl / icon / iconColor）", () => {
    for (const preset of CODEX_PROVIDER_PRESETS) {
      expect(preset.websiteUrl).toBeTruthy()
      expect(preset.icon).toBeTruthy()
      expect(preset.iconColor).toBeTruthy()
    }
  })
})

describe("GEMINI_PROVIDER_PRESETS", () => {
  it("总数 6 且名称与顺序与需求清单完全一致", () => {
    expect(GEMINI_PROVIDER_PRESETS).toHaveLength(6)
    expect(GEMINI_PROVIDER_PRESETS.map((p) => p.name)).toEqual(
      EXPECTED_GEMINI_NAMES,
    )
  })

  it("名称唯一", () => {
    const names = GEMINI_PROVIDER_PRESETS.map((p) => p.name)
    expect(new Set(names).size).toBe(names.length)
  })

  it("category 分组合计为 官方 2 / 聚合 4", () => {
    const counts: Record<ProviderCategory, number> = {
      official: 0,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 0,
      custom: 0,
    }
    for (const preset of GEMINI_PROVIDER_PRESETS) {
      counts[preset.category] += 1
    }
    expect(counts).toEqual({
      official: 2,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 4,
      custom: 0,
    })
  })

  it("每项 settingsConfig 是合法 JSON 对象", () => {
    for (const preset of GEMINI_PROVIDER_PRESETS) {
      const parsed: unknown = JSON.parse(preset.settingsConfig)
      expect(
        parsed !== null && typeof parsed === "object" && !Array.isArray(parsed),
      ).toBe(true)
    }
  })

  it("Google Gemini 登录态版无凭据（无 GEMINI_API_KEY）", () => {
    const oauth = GEMINI_PROVIDER_PRESETS.find(
      (p) => p.name === "Google Gemini",
    )
    expect(oauth).toBeDefined()
    // 登录态版 env 为空且无 config → settingsConfig 折叠为 "{}"，与 Rust
    // parse_gemini_settings 对空快照的宽容契约一致（缺 env 当空 env）。
    const parsed = JSON.parse(oauth!.settingsConfig) as {
      env?: Record<string, string>
    }
    expect(parsed.env ?? {}).toEqual({})
    expect(parsed.env?.GEMINI_API_KEY).toBeUndefined()
  })

  it("E-FlowCode 的 config 含 general.previewFeatures 与 sessionRetention 全字段", () => {
    const eflow = GEMINI_PROVIDER_PRESETS.find((p) => p.name === "E-FlowCode")
    expect(eflow).toBeDefined()
    const parsed = JSON.parse(eflow!.settingsConfig) as {
      config?: {
        general?: {
          previewFeatures?: boolean
          sessionRetention?: {
            enabled?: boolean
            maxAge?: string
            warningAcknowledged?: boolean
          }
        }
      }
    }
    expect(parsed.config?.general?.previewFeatures).toBe(true)
    expect(parsed.config?.general?.sessionRetention).toEqual({
      enabled: true,
      maxAge: "30d",
      warningAcknowledged: true,
    })
  })

  it("每个预设都带必填元数据（websiteUrl / icon / iconColor）", () => {
    for (const preset of GEMINI_PROVIDER_PRESETS) {
      expect(preset.websiteUrl).toBeTruthy()
      expect(preset.icon).toBeTruthy()
      expect(preset.iconColor).toBeTruthy()
    }
  })
})

describe("GROK_PROVIDER_PRESETS", () => {
  it("总数 6 且名称与顺序与需求清单完全一致", () => {
    expect(GROK_PROVIDER_PRESETS).toHaveLength(6)
    expect(GROK_PROVIDER_PRESETS.map((p) => p.name)).toEqual(
      EXPECTED_GROK_NAMES,
    )
  })

  it("名称唯一", () => {
    const names = GROK_PROVIDER_PRESETS.map((p) => p.name)
    expect(new Set(names).size).toBe(names.length)
  })

  it("category 分组合计为 官方 2 / 聚合 4", () => {
    const counts: Record<ProviderCategory, number> = {
      official: 0,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 0,
      custom: 0,
    }
    for (const preset of GROK_PROVIDER_PRESETS) {
      counts[preset.category] += 1
    }
    expect(counts).toEqual({
      official: 2,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 4,
      custom: 0,
    })
  })

  it("每项 settingsConfig 是合法 JSON 对象", () => {
    for (const preset of GROK_PROVIDER_PRESETS) {
      const parsed: unknown = JSON.parse(preset.settingsConfig)
      expect(
        parsed !== null && typeof parsed === "object" && !Array.isArray(parsed),
      ).toBe(true)
    }
  })

  it('Grok Official 是登录态版（settingsConfig 为 "{}"）', () => {
    const official = GROK_PROVIDER_PRESETS.find(
      (p) => p.name === "Grok Official",
    )
    expect(official).toBeDefined()
    expect(official!.settingsConfig).toBe("{}")
  })

  it("非官方预设用 [model.cc-one] 命名 profile 格式，非 codex 风格", () => {
    // Grok 的 config.toml 是命名 profile 式（[models].default + [model.<name>]），
    // 不是 codex 的 model_provider / model_providers。CC-Switch 的 grok 预设误用
    // 了 codex 风格——本测试锁死正确格式，防止回退到那个 bug。
    for (const preset of GROK_PROVIDER_PRESETS) {
      if (preset.name === "Grok Official") continue
      const parsed = JSON.parse(preset.settingsConfig) as { config?: string }
      expect(parsed.config, `${preset.name} 应带 config TOML`).toBeTruthy()
      expect(
        parsed.config!.includes("[model.cc-one]"),
        `${preset.name} 必须用 [model.cc-one] profile`,
      ).toBe(true)
      expect(
        parsed.config!.includes("model_providers"),
        `${preset.name} 不得用 codex 风格 model_providers`,
      ).toBe(false)
    }
  })

  it("每个预设都带必填元数据（websiteUrl / icon / iconColor）", () => {
    for (const preset of GROK_PROVIDER_PRESETS) {
      expect(preset.websiteUrl).toBeTruthy()
      expect(preset.icon).toBeTruthy()
      expect(preset.iconColor).toBeTruthy()
    }
  })
})

describe("OPENCODE_PROVIDER_PRESETS", () => {
  /** 权威清单：名称与顺序不得增删改（官方 3 + 国内大厂 6 + 热门聚合 3）。 */
  const EXPECTED_OPENCODE_NAMES = [
    "OpenAI",
    "Anthropic",
    "Google Gemini",
    "DeepSeek",
    "Kimi",
    "Zhipu GLM",
    "阿里百炼",
    "火山方舟",
    "MiniMax",
    "OpenRouter",
    "千象 Qiniu",
    "AIHubMix",
  ]

  it("总数 12 且名称与顺序与需求清单完全一致", () => {
    expect(OPENCODE_PROVIDER_PRESETS).toHaveLength(12)
    expect(OPENCODE_PROVIDER_PRESETS.map((p) => p.name)).toEqual(
      EXPECTED_OPENCODE_NAMES,
    )
  })

  it("名称唯一", () => {
    const names = OPENCODE_PROVIDER_PRESETS.map((p) => p.name)
    expect(new Set(names).size).toBe(names.length)
  })

  it("category 分组合计为 官方 3 / 国内 6 / 聚合 3", () => {
    const counts: Record<ProviderCategory, number> = {
      official: 0,
      cloud_provider: 0,
      cn_official: 0,
      aggregator: 0,
      custom: 0,
    }
    for (const preset of OPENCODE_PROVIDER_PRESETS) {
      counts[preset.category] += 1
    }
    expect(counts).toEqual({
      official: 3,
      cloud_provider: 0,
      cn_official: 6,
      aggregator: 3,
      custom: 0,
    })
  })

  it("每项 settingsConfig 经 parseOpenCodeConfig 读出 npm + baseURL（OpenCode entry 形状）", () => {
    for (const preset of OPENCODE_PROVIDER_PRESETS) {
      const cfg = parseOpenCodeConfig(preset.settingsConfig)
      expect(cfg.npm, `${preset.name} 应带 @ai-sdk/* 包名`).toMatch(
        /^@ai-sdk\//,
      )
      expect(cfg.baseURL, `${preset.name} 应带 baseURL`).toBeTruthy()
      // apiKey 占位为空——密钥由表单填，不进预设。
      expect(cfg.apiKey).toBe("")
    }
  })

  it("每个预设都带必填元数据（websiteUrl / icon / iconColor）", () => {
    for (const preset of OPENCODE_PROVIDER_PRESETS) {
      expect(preset.websiteUrl).toBeTruthy()
      expect(preset.icon).toBeTruthy()
      expect(preset.iconColor).toBeTruthy()
    }
  })
})

describe("presetsForApp", () => {
  it("claude 返回 18 个 Claude 预设数组", () => {
    expect(presetsForApp("claude")).toBe(PROVIDER_PRESETS)
    expect(presetsForApp("claude")).toHaveLength(18)
  })

  it("codex 返回 17 个 Codex 预设数组", () => {
    expect(presetsForApp("codex")).toBe(CODEX_PROVIDER_PRESETS)
    expect(presetsForApp("codex")).toHaveLength(17)
  })

  it("gemini 返回 6 个 Gemini 预设数组", () => {
    expect(presetsForApp("gemini")).toBe(GEMINI_PROVIDER_PRESETS)
    expect(presetsForApp("gemini")).toHaveLength(6)
  })

  it("grok 返回 6 个 Grok 预设数组", () => {
    expect(presetsForApp("grok")).toBe(GROK_PROVIDER_PRESETS)
    expect(presetsForApp("grok")).toHaveLength(6)
  })

  it("opencode 返回 12 个 OpenCode 预设数组", () => {
    expect(presetsForApp("opencode")).toBe(OPENCODE_PROVIDER_PRESETS)
    expect(presetsForApp("opencode")).toHaveLength(12)
  })

  it("五个 app 的返回值互不相同（应用维度分派，不串池）", () => {
    const claude = presetsForApp("claude")
    const codex = presetsForApp("codex")
    const gemini = presetsForApp("gemini")
    const grok = presetsForApp("grok")
    const opencode = presetsForApp("opencode")
    expect(claude).not.toBe(codex)
    expect(claude).not.toBe(gemini)
    expect(claude).not.toBe(grok)
    expect(claude).not.toBe(opencode)
    expect(codex).not.toBe(gemini)
    expect(codex).not.toBe(grok)
    expect(codex).not.toBe(opencode)
    expect(gemini).not.toBe(grok)
    expect(gemini).not.toBe(opencode)
    expect(grok).not.toBe(opencode)
  })

  it("覆盖所有 App 类型（类型已约束，穷尽即可）", () => {
    const apps: App[] = ["claude", "codex", "gemini", "grok", "opencode"]
    for (const app of apps) {
      expect(presetsForApp(app).length).toBeGreaterThan(0)
    }
  })
})
