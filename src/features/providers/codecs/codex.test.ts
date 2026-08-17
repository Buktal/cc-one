import { describe, expect, it } from "vitest"
import { CODEX_PROVIDER_PRESETS } from "@/features/providers/codex-presets"
import {
  codexApiKey,
  codexConfigToml,
  parseCodexConfig,
  withCodexApiKey,
  withCodexConfigToml,
} from "./codex"

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
    // OpenAI (ChatGPT 登录) 是登录态版（settingsConfig = "{}"）。
    const official = CODEX_PROVIDER_PRESETS.find(
      (p) => p.name === "OpenAI (ChatGPT 登录)",
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
