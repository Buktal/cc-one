import { describe, expect, it } from "vitest"
import { sessionAgentKind, sessionSourceLabel } from "./source-labels"

describe("sessionSourceLabel", () => {
  it("maps known source tags to display names", () => {
    expect(sessionSourceLabel("claude_code")).toBe("Claude Code")
    expect(sessionSourceLabel("codex_cli")).toBe("Codex CLI")
  })

  it("falls through unknown tags verbatim and empty to em dash", () => {
    expect(sessionSourceLabel("some_new_source")).toBe("some_new_source")
    expect(sessionSourceLabel("")).toBe("—")
  })
})

describe("sessionAgentKind", () => {
  it("classifies an empty tag as main session", () => {
    expect(sessionAgentKind("")).toEqual({ kind: "main" })
  })

  it("classifies a non-empty tag as a subagent with its type", () => {
    expect(sessionAgentKind("Explore")).toEqual({
      kind: "subagent",
      type: "Explore",
    })
    expect(sessionAgentKind("agent")).toEqual({
      kind: "subagent",
      type: "agent",
    })
  })
})
