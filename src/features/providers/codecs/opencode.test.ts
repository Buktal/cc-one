import { describe, expect, it } from "vitest"
import {
  openCodeApiKey,
  openCodeBaseUrl,
  openCodeHeaders,
  openCodeModels,
  openCodeNpm,
  parseOpenCodeConfig,
  withOpenCodeApiKey,
  withOpenCodeBaseUrl,
  withOpenCodeHeaders,
  withOpenCodeModels,
  withOpenCodeNpm,
} from "./opencode"

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
