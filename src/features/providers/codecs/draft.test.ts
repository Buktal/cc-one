// biome-ignore-all lint/suspicious/noTemplateCurlyInString: 模板变量占位符文本
import { describe, expect, it } from "vitest"
import { finalizeDraft, seedDraftText } from "./draft"

describe("seedDraftText", () => {
  it("非 claude 恒等——空草稿 {} 不被补出 env 键（缺陷回归锁）", () => {
    for (const app of ["codex", "gemini", "grok", "opencode"] as const) {
      expect(seedDraftText(app, "{}")).toBe("{}")
    }
  })

  it("非 claude 预设快照原样——不追加任何字段", () => {
    const presetSnapshot = JSON.stringify({
      auth: { OPENAI_API_KEY: "sk-1" },
      config: 'model = "m"',
    })
    expect(seedDraftText("codex", presetSnapshot)).toBe(presetSnapshot)
    const geminiSnapshot = JSON.stringify({
      env: { GEMINI_API_KEY: "g-key" },
    })
    expect(seedDraftText("gemini", geminiSnapshot)).toBe(geminiSnapshot)
  })

  it("claude 空快照建出 env 容器，空角色模型无标记可写", () => {
    expect(JSON.parse(seedDraftText("claude", "{}"))).toEqual({ env: {} })
  })

  it("claude 有主模型时给支持 1M 的角色补 [1M] 标记（回退链），haiku 不动", () => {
    const seeded = seedDraftText(
      "claude",
      JSON.stringify({ env: { ANTHROPIC_MODEL: "claude-opus-5" } }),
    )
    const env = JSON.parse(seeded).env as Record<string, string>
    // 主模型经回退链进各角色并带标记；fable/subagent 经已被标记的角色回退，
    // setModelOneM 先剥后加保证幂等。
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("claude-opus-5[1M]")
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe("claude-opus-5[1M]")
    expect(env.ANTHROPIC_DEFAULT_FABLE_MODEL).toBe("claude-opus-5[1M]")
    expect(env.ANTHROPIC_DEFAULT_SUBAGENT_MODEL).toBe("claude-opus-5[1M]")
    expect(env.ANTHROPIC_DEFAULT_HAIKU_MODEL).toBeUndefined()
    expect(env.ANTHROPIC_SMALL_FAST_MODEL).toBeUndefined()
    expect(seeded).toContain('"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"')
  })

  it("claude 已带标记的模型不再叠标记", () => {
    const seeded = seedDraftText(
      "claude",
      JSON.stringify({ env: { ANTHROPIC_MODEL: "m[1m]" } }),
    )
    const env = JSON.parse(seeded).env as Record<string, string>
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe("m[1M]")
  })
})

describe("finalizeDraft", () => {
  it("非 claude 直通：configText 与 meta 原样返回", () => {
    const text = JSON.stringify({ auth: { OPENAI_API_KEY: "sk" } })
    const result = finalizeDraft("codex", text, {}, '{"custom":"x"}')
    expect(result).toEqual({
      ok: true,
      settingsConfig: text,
      meta: '{"custom":"x"}',
    })
  })

  it("claude 物化模板变量并记录进 meta", () => {
    const result = finalizeDraft(
      "claude",
      JSON.stringify({
        env: { ANTHROPIC_BASE_URL: "https://x/${REGION}/api" },
      }),
      { REGION: "us-east-1" },
      "{}",
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    const config = JSON.parse(result.settingsConfig) as {
      env: Record<string, string>
    }
    expect(config.env.ANTHROPIC_BASE_URL).toBe("https://x/us-east-1/api")
    expect(JSON.parse(result.meta)).toEqual({
      templateValues: { REGION: "us-east-1" },
    })
  })

  it("claude 保存时归一基础字段：端点收尾 trim、空端点清键", () => {
    const result = finalizeDraft(
      "claude",
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: " https://a.example ",
          ANTHROPIC_AUTH_TOKEN: "sk",
        },
      }),
      {},
      "{}",
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    const env = (
      JSON.parse(result.settingsConfig) as { env: Record<string, string> }
    ).env
    expect(env.ANTHROPIC_BASE_URL).toBe("https://a.example")
    expect(env.ANTHROPIC_AUTH_TOKEN).toBe("sk")
  })

  it("claude 有 ${VAR} 未填值时拒绝保存并列出变量名", () => {
    const result = finalizeDraft(
      "claude",
      JSON.stringify({
        env: { ANTHROPIC_BASE_URL: "https://${REGION}/", notes: "${EXTRA}" },
      }),
      {},
      "{}",
    )
    expect(result).toEqual({ ok: false, unfilled: ["REGION", "EXTRA"] })
  })

  it("claude meta 保留未知键；变量值为空时移除 templateValues 键", () => {
    const result = finalizeDraft(
      "claude",
      JSON.stringify({ env: {} }),
      { GONE: "" },
      JSON.stringify({ liveManaged: true, templateValues: { OLD: "v" } }),
    )
    expect(result.ok).toBe(true)
    if (!result.ok) return
    expect(JSON.parse(result.meta)).toEqual({ liveManaged: true })
  })
})
