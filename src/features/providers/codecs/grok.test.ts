import { describe, expect, it } from "vitest"
import { GROK_PROVIDER_PRESETS } from "@/features/providers/grok-presets"
import { grokConfigToml, parseGrokConfig, withGrokConfigToml } from "./grok"

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
