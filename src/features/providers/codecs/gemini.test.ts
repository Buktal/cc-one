import { describe, expect, it } from "vitest"
import { GEMINI_PROVIDER_PRESETS } from "@/features/providers/gemini-presets"
import {
  geminiApiKey,
  geminiBaseUrl,
  geminiModel,
  parseGeminiConfig,
  withGeminiConfigJson,
  withGeminiEnv,
} from "./gemini"

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
