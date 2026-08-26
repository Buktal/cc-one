import { describe, expect, it } from "vitest"
import { APP_PROFILES, snippetSupportLanguage } from "./app-profiles"
import { PROVIDER_PRESETS } from "./presets"

describe("APP_PROFILES", () => {
  it("附加模式只有 opencode；其余单激活", () => {
    expect(
      Object.entries(APP_PROFILES)
        .filter(([, p]) => p.additive)
        .map(([app]) => app),
    ).toEqual(["opencode"])
  })

  it("片段支持形态：claude/gemini 在 settings_config 层、codex/grok 在写盘层、opencode 无", () => {
    expect(APP_PROFILES.claude.snippet.kind).toBe("settings-config")
    expect(APP_PROFILES.gemini.snippet.kind).toBe("settings-config")
    expect(APP_PROFILES.codex.snippet.kind).toBe("write-layer")
    expect(APP_PROFILES.grok.snippet.kind).toBe("write-layer")
    expect(APP_PROFILES.opencode.snippet.kind).toBe("none")
  })

  it("编辑器语言随 snippet 形态派生（写盘层 TOML、settings_config 层 JSON）", () => {
    for (const profile of Object.values(APP_PROFILES)) {
      if (profile.snippet.kind === "none") continue
      expect(snippetSupportLanguage(profile.snippet)).toBe(
        profile.snippet.kind === "write-layer" ? "toml" : "json",
      )
    }
  })

  it("draftIssue 只有 gemini 有（凭据/结构警告）", () => {
    expect(
      Object.values(APP_PROFILES).filter(
        (p) =>
          p.snippet.kind === "settings-config" &&
          p.snippet.draftIssue !== undefined,
      ).length,
    ).toBe(1)
    const issue = APP_PROFILES.gemini.snippet
    if (issue.kind !== "settings-config" || !issue.draftIssue) return
    // gemini 凭据键经 env 检出（镜像后端 is_sensitive_config_key）。
    expect(issue.draftIssue('{"env": {"GEMINI_API_KEY": "k"}}')).toBe(
      "env.GEMINI_API_KEY",
    )
  })

  it("liveFile 与后端 live_paths 的文件名一致", () => {
    expect(APP_PROFILES.claude.liveFile).toBe("settings.json")
    expect(APP_PROFILES.codex.liveFile).toBe("config.toml")
    expect(APP_PROFILES.gemini.liveFile).toBe(".env")
    expect(APP_PROFILES.grok.liveFile).toBe("config.toml")
    expect(APP_PROFILES.opencode.liveFile).toBe("opencode.json")
  })

  it("新建空草稿形状：claude 是 env 容器，其余 {}。", () => {
    expect(APP_PROFILES.claude.newDraftText).toBe('{\n  "env": {}\n}')
    for (const app of ["codex", "gemini", "grok", "opencode"] as const) {
      expect(APP_PROFILES[app].newDraftText).toBe("{}")
    }
  })
})

describe("modelFetch 参数提取（app-profiles 行）", () => {
  it("codex / grok 无拉模型入口", () => {
    for (const app of ["codex", "grok"] as const) {
      expect(APP_PROFILES[app].modelFetch).toBeNull()
    }
  })

  it("claude：端点与 key 必填，缺哪个报哪个", () => {
    const fetch = APP_PROFILES.claude.modelFetch
    if (!fetch) throw new Error("claude 应有 modelFetch")
    expect(fetch('{"env": {}}')).toEqual({ ok: false, missing: "endpoint" })
    expect(
      fetch(JSON.stringify({ env: { ANTHROPIC_BASE_URL: "https://a" } })),
    ).toEqual({ ok: false, missing: "key" })
  })

  it("claude：端点命中预设默认值时带该预设的 modelsUrl 覆写", () => {
    const preset = PROVIDER_PRESETS.find((p) => p.modelsUrl)
    if (!preset) return
    const endpoint = (
      JSON.parse(preset.settingsConfig) as { env: Record<string, string> }
    ).env.ANTHROPIC_BASE_URL
    const result = APP_PROFILES.claude.modelFetch?.(
      JSON.stringify({
        env: { ANTHROPIC_BASE_URL: endpoint, ANTHROPIC_AUTH_TOKEN: "sk" },
      }),
    )
    expect(result).toEqual({
      ok: true,
      args: {
        app: "claude",
        baseUrl: endpoint,
        apiKey: "sk",
        modelsUrl: preset.modelsUrl ?? null,
      },
    })
  })

  it("gemini：key 唯一必填，端点可空、modelsUrl 恒 null", () => {
    const fetch = APP_PROFILES.gemini.modelFetch
    if (!fetch) throw new Error("gemini 应有 modelFetch")
    expect(fetch("{}")).toEqual({ ok: false, missing: "key" })
    expect(fetch('{"env": {"GEMINI_API_KEY": "g"}}')).toEqual({
      ok: true,
      args: { app: "gemini", baseUrl: "", apiKey: "g", modelsUrl: null },
    })
  })

  it("opencode：端点与 key 均必填（options 形状读取）", () => {
    const fetch = APP_PROFILES.opencode.modelFetch
    if (!fetch) throw new Error("opencode 应有 modelFetch")
    expect(fetch("{}")).toEqual({ ok: false, missing: "endpoint" })
    expect(
      fetch(
        JSON.stringify({
          options: { baseURL: "https://a", apiKey: "ok" },
        }),
      ),
    ).toEqual({
      ok: true,
      args: {
        app: "opencode",
        baseUrl: "https://a",
        apiKey: "ok",
        modelsUrl: null,
      },
    })
  })
})
