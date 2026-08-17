import { describe, expect, it } from "vitest"
import {
  findSensitiveConfigKey,
  geminiSnippetIssue,
  geminiSnippetMissingKeys,
  groupSnippetCandidates,
  isSensitiveConfigKey,
  pairModelNameKeys,
  snippetCoveredKeys,
  snippetMissingKeys,
} from "./snippet"

describe("snippetMissingKeys", () => {
  const snippet = JSON.stringify({ includeCoAuthoredBy: false })

  it("reports controlled keys the config does not have", () => {
    // 配置里没有 includeCoAuthoredBy → 片段会在写盘时补上它。
    expect(snippetMissingKeys('{"env":{}}', snippet)).toEqual([
      "includeCoAuthoredBy",
    ])
  })

  it("reports env only when at least one snippet env key is missing", () => {
    const cfg = JSON.stringify({ env: { ANTHROPIC_MODEL: "m" } })
    expect(
      snippetMissingKeys(cfg, JSON.stringify({ env: { A: "1" } })),
    ).toEqual(["env"])
    // 片段 env 全被配置覆盖 → 空。
    expect(
      snippetMissingKeys(
        cfg,
        JSON.stringify({ env: { ANTHROPIC_MODEL: "snippet-default" } }),
      ),
    ).toEqual([])
  })

  it("ignores snippet keys that are not controlled fields", () => {
    const bad = JSON.stringify({
      permissions: { deny: ["Bash"] },
      hooks: { PostToolUse: [] },
      model: "claude-opus-4-5",
    })
    expect(snippetMissingKeys('{"env":{}}', bad)).toEqual([])
  })

  it("reports only the keys actually missing (subset)", () => {
    const cfg = JSON.stringify({ includeCoAuthoredBy: true, env: {} })
    const snip = JSON.stringify({
      includeCoAuthoredBy: false,
      skipWebFetchPreflight: true,
    })
    expect(snippetMissingKeys(cfg, snip)).toEqual(["skipWebFetchPreflight"])
  })

  it("returns empty for empty or garbage input", () => {
    expect(snippetMissingKeys("", "")).toEqual([])
    expect(snippetMissingKeys("{nope", snippet)).toEqual([])
    expect(snippetMissingKeys('{"env":{}}', "{nope")).toEqual([])
  })
})

describe("isSensitiveConfigKey", () => {
  // 用例与后端 `sensitive_keys_detected` 逐字一致（ADR-0010：前后端判定一致）。
  it("detects credential keys (token / key / secret / password)", () => {
    for (const key of [
      "ANTHROPIC_AUTH_TOKEN",
      "ANTHROPIC_API_KEY",
      "GEMINI_API_KEY",
      "GOOGLE_API_KEY", // CC-Switch 泄漏事故的同款键
      "APIKEY",
      "API_KEY",
      "TOKEN",
      "SECRET",
      "OPENAI_API_KEY",
      "MY_PRIVATE_KEY",
      "DB_PASSWORD",
      "SERVICE_ACCOUNT_CREDENTIALS",
    ]) {
      expect(isSensitiveConfigKey(key), `${key} 应判为凭据`).toBe(true)
    }
  })

  // 用例与后端 `non_sensitive_keys_pass_through` 逐字一致。
  it("passes through non-credential keys (model / endpoint / flags)", () => {
    for (const key of [
      "ANTHROPIC_MODEL",
      "ANTHROPIC_BASE_URL",
      "ANTHROPIC_DEFAULT_FABLE_MODEL",
      "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
      "CLAUDE_CODE_EFFORT_LEVEL",
      "CLAUDE_CODE_SUBAGENT_MODEL",
      "CLAUDE_CODE_ATTRIBUTION_HEADER",
      "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
      "GEMINI_MODEL",
      "MY_FLAG",
    ]) {
      expect(isSensitiveConfigKey(key), `${key} 不应判为凭据`).toBe(false)
    }
  })

  it("is case-insensitive", () => {
    expect(isSensitiveConfigKey("anthropic_auth_token")).toBe(true)
    expect(isSensitiveConfigKey("OpenAI_Api_Key")).toBe(true)
    expect(isSensitiveConfigKey("anthropic_model")).toBe(false)
  })
})

describe("findSensitiveConfigKey", () => {
  it("scans top-level and env.* keys", () => {
    expect(findSensitiveConfigKey({ env: { GEMINI_API_KEY: "k" } })).toBe(
      "env.GEMINI_API_KEY",
    )
    expect(findSensitiveConfigKey({ apiKey: "x" })).toBe("apiKey")
    // 非凭据键 → null。
    expect(findSensitiveConfigKey({ env: { GEMINI_MODEL: "m" } })).toBeNull()
    expect(findSensitiveConfigKey({})).toBeNull()
  })
})

describe("geminiSnippetIssue", () => {
  it("detects credential key, endpoint key, and clean snippets", () => {
    // 凭据键（env 下）。
    expect(geminiSnippetIssue('{"env": {"GEMINI_API_KEY": "k"}}')).toBe(
      "env.GEMINI_API_KEY",
    )
    // 端点键（env 下——与后端 validate_gemini_extras 同款只扫 env）。
    expect(geminiSnippetIssue('{"env": {"GOOGLE_GEMINI_BASE_URL": "u"}}')).toBe(
      "env.GOOGLE_GEMINI_BASE_URL",
    )
    // env 以外的顶层键一律零效果（合并层只认 env 子对象）——扁平键与其它
    // 顶层键都指给用户（与后端 validate_snippet gemini 分支同判）。
    expect(geminiSnippetIssue('{"GEMINI_MODEL": "m"}')).toBe("GEMINI_MODEL")
    expect(geminiSnippetIssue('{"includeCoAuthoredBy": false}')).toBe(
      "includeCoAuthoredBy",
    )
    // 合法：非凭据、非端点。
    expect(
      geminiSnippetIssue('{"env": {"GEMINI_MODEL": "gemini-2.5-flash"}}'),
    ).toBeNull()
    // 空 / 垃圾草稿 → null（不误导）。
    expect(geminiSnippetIssue("")).toBeNull()
    expect(geminiSnippetIssue("{nope")).toBeNull()
  })
})

describe("geminiSnippetMissingKeys", () => {
  const cfg = '{"env": {"GEMINI_MODEL": "gemini-2.5-flash"}}'

  it("reports snippet env keys missing from the config env", () => {
    const snippet =
      '{"env": {"GEMINI_MODEL": "x", "GEMINI_TEMPERATURE": "0.7"}}'
    expect(geminiSnippetMissingKeys(cfg, snippet)).toEqual([
      "GEMINI_TEMPERATURE",
    ])
  })

  it("returns [] when the config already covers the snippet", () => {
    expect(
      geminiSnippetMissingKeys(cfg, '{"env": {"GEMINI_MODEL": "x"}}'),
    ).toEqual([])
  })

  it("ignores non-env snippet keys (only env is merged at the settings layer)", () => {
    const snippet = '{"env": {"GEMINI_MODEL": "x"}, "mcpServers": {}}'
    expect(geminiSnippetMissingKeys(cfg, snippet)).toEqual([])
  })

  it("is conservative on unparseable input", () => {
    expect(geminiSnippetMissingKeys("", "")).toEqual([])
    expect(geminiSnippetMissingKeys("{nope", '{"env": {"A": "1"}}')).toEqual([])
    expect(geminiSnippetMissingKeys(cfg, "{nope")).toEqual([])
  })
})

describe("snippetCoveredKeys", () => {
  it("collects top-level and env keys for JSON apps", () => {
    const covered = snippetCoveredKeys(
      "claude",
      '{"env": {"ANTHROPIC_MODEL": "m"}, "includeCoAuthoredBy": false}',
    )
    expect(covered.has("env")).toBe(true)
    expect(covered.has("ANTHROPIC_MODEL")).toBe(true)
    expect(covered.has("includeCoAuthoredBy")).toBe(true)
  })

  it("collects top-level tables and scalar keys for TOML apps", () => {
    const covered = snippetCoveredKeys(
      "grok",
      '[tui]\ntheme = "dark"\n\n[mcp_servers.github]\ncommand = "npx"',
    )
    expect(covered.has("tui")).toBe(true)
    expect(covered.has("mcp_servers")).toBe(true)
    // 表内标量不算顶层键（T6 候选是顶层键）。
    expect(covered.has("theme")).toBe(false)
  })

  it("returns an empty set for empty / unparseable snippets", () => {
    expect(snippetCoveredKeys("codex", "")).toEqual(new Set())
    expect(snippetCoveredKeys("claude", "{nope")).toEqual(new Set())
  })
})

describe("groupSnippetCandidates", () => {
  it("把候选按 端点 / 模型 / 行为开关 三组归类，组内保序", () => {
    expect(
      groupSnippetCandidates([
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "includeCoAuthoredBy",
        "CLAUDE_CODE_SUBAGENT_MODEL",
        "effortLevel",
      ]),
    ).toEqual({
      endpoint: ["ANTHROPIC_BASE_URL"],
      model: ["ANTHROPIC_DEFAULT_FABLE_MODEL", "CLAUDE_CODE_SUBAGENT_MODEL"],
      behavior: ["includeCoAuthoredBy", "effortLevel"],
    })
  })

  it("空候选 → 三组皆空", () => {
    expect(groupSnippetCandidates([])).toEqual({
      endpoint: [],
      model: [],
      behavior: [],
    })
  })
})

describe("pairModelNameKeys", () => {
  it("把 *_MODEL 与对应 *_MODEL_NAME 配对相邻（MODEL 在前），无配对键保序", () => {
    expect(
      pairModelNameKeys([
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
        "CLAUDE_CODE_SUBAGENT_MODEL",
      ]),
    ).toEqual([
      "ANTHROPIC_DEFAULT_SONNET_MODEL",
      "ANTHROPIC_DEFAULT_FABLE_MODEL",
      "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
      "ANTHROPIC_MODEL",
      "CLAUDE_CODE_SUBAGENT_MODEL",
    ])
  })

  it("NAME 在 MODEL 前出现时仍配对（挪到 MODEL 后）", () => {
    expect(
      pairModelNameKeys([
        "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_MODEL",
      ]),
    ).toEqual([
      "ANTHROPIC_DEFAULT_HAIKU_MODEL",
      "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
      "ANTHROPIC_MODEL",
    ])
  })

  it("孤儿 _MODEL_NAME（无对应 _MODEL）原样保留", () => {
    expect(pairModelNameKeys(["ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"])).toEqual([
      "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
    ])
  })

  it("空数组 → 空", () => {
    expect(pairModelNameKeys([])).toEqual([])
  })
})
