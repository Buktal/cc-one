// Tests for the conversation turn structure (architecture.md: "测试必须跑生产
// 路径"). The attachment rule is the load-bearing invariant: tool runs join the
// AI message card that issued them — nearest preceding assistant row within the
// turn, else the turn's first assistant row — never interleaving as standalone
// rows, and never leaking across a turn boundary (a new user message starts a
// clean slate).

import { describe, expect, it } from "vitest"

import type { SessionMessage } from "@/types/generated/bindings"

import {
  conversationLayout,
  groupConversation,
  ownsFlowRow,
  toolSummary,
  toolTurnCount,
  userTurnCount,
} from "./conversation"

let seq = 0

/** Build one transcript row. Tool rows carry a name + JSON-ish content; the
 *  rest are plain text rows. */
function row(
  role: SessionMessage["role"],
  uuid: string,
  content: string,
  name?: string,
): SessionMessage {
  return {
    uuid,
    session_id: "s",
    role,
    ts: `2026-08-19T00:00:${String(seq++).padStart(2, "0")}Z`,
    ...(role === "assistant" ? { model: "glm-5.2" } : {}),
    ...(name !== undefined ? { name } : {}),
    content,
  }
}

describe("groupConversation", () => {
  it("groups rows into turns sliced at user messages, prelude numbered 0", () => {
    // Prelude: an assistant row before any user message.
    const turns = groupConversation([
      row("system", "sys", "context loaded"),
      row("user", "u1", "first question"),
      row("assistant", "a1", "first answer"),
      row("user", "u2", "second question"),
      row("assistant", "a2", "second answer"),
    ])
    expect(turns.map((t) => [t.number, t.user?.uuid ?? null])).toEqual([
      [0, null],
      [1, "u1"],
      [2, "u2"],
    ])
    // Prelude keeps its rows; turns keep theirs in order.
    expect(
      turns[0].nodes.map((n) => n.kind === "message" && n.message.uuid),
    ).toEqual(["sys"])
    expect(
      turns[1].nodes.map((n) => n.kind === "message" && n.message.uuid),
    ).toEqual(["a1"])
    expect(
      turns[2].nodes.map((n) => n.kind === "message" && n.message.uuid),
    ).toEqual(["a2"])
  })

  it("numbers turns the way the turn-nav panel does (user-message ordinal)", () => {
    const turns = groupConversation([
      row("user", "u1", "q1"),
      row("assistant", "a1", "r1"),
      row("user", "u2", "q2"),
      row("assistant", "a2", "r2"),
    ])
    expect(turns.map((t) => t.number)).toEqual([1, 2])
  })

  it("attaches a tool run to the nearest preceding assistant row", () => {
    // Real Claude shape: text("let me look") and tool_use are SEPARATE events
    // of the same flow — the tool run follows its issuing text row.
    const turns = groupConversation([
      row("user", "u1", "check the build"),
      row("assistant", "a1", "let me look"),
      row("tool", "t1", '{"command":"cargo test"}', "Bash"),
      row("tool", "t2", '{"file_path":"src/main.rs"}', "Read"),
      row("assistant", "a2", "build is green"),
    ])
    expect(turns).toHaveLength(1)
    const nodes = turns[0].nodes
    expect(nodes[0]).toMatchObject({ kind: "message", message: { uuid: "a1" } })
    expect(nodes[0].tools.map((t) => t.uuid)).toEqual(["t1", "t2"])
    expect(nodes[1]).toMatchObject({
      kind: "message",
      message: { uuid: "a2" },
      tools: [],
    })
  })

  it("attaches a turn-opening tool run to the turn's first assistant row below", () => {
    // A turn that opens straight with tool calls (the model acted without
    // narrating first): the run joins the first assistant row that follows.
    const turns = groupConversation([
      row("user", "u1", "run it"),
      row("tool", "t1", '{"command":"ls"}', "Bash"),
      row("tool", "t2", '{"command":"pwd"}', "Bash"),
      row("assistant", "a1", "done"),
    ])
    expect(turns[0].nodes).toHaveLength(1)
    expect(turns[0].nodes[0]).toMatchObject({
      kind: "message",
      message: { uuid: "a1" },
    })
    expect(turns[0].nodes[0].tools.map((t) => t.uuid)).toEqual(["t1", "t2"])
  })

  it("keeps a tool run as a standalone node when the turn has no assistant row", () => {
    const turns = groupConversation([
      row("user", "u1", "hello?"),
      row("tool", "t1", '{"command":"ls"}', "Bash"),
      row("user", "u2", "again"),
      row("assistant", "a1", "hi"),
    ])
    // Turn 1: only a loose tools node; turn 2 untouched by turn 1's tools.
    expect(turns[0].nodes).toEqual([
      { kind: "tools", tools: [expect.objectContaining({ uuid: "t1" })] },
    ])
    expect(turns[1].nodes[0]).toMatchObject({
      kind: "message",
      message: { uuid: "a1" },
      tools: [],
    })
  })

  it("never attaches tools across a turn boundary", () => {
    // The run follows an assistant row, but a new user message intervenes —
    // the tools belong to the NEW turn's flow and must not hang off the
    // previous turn's assistant row.
    const turns = groupConversation([
      row("user", "u1", "q1"),
      row("assistant", "a1", "r1"),
      row("user", "u2", "q2"),
      row("tool", "t1", '{"command":"ls"}', "Bash"),
      row("assistant", "a2", "r2"),
    ])
    expect(turns[0].nodes[0].tools).toEqual([])
    expect(turns[1].nodes[0].tools.map((t) => t.uuid)).toEqual(["t1"])
  })

  it("splits two runs around a middle assistant row (run attaches backward, not forward)", () => {
    // [a1][run1][a2]: run1 issued after a1's text — attach backward to a1,
    // so one API message's blocks never scatter across two cards.
    const turns = groupConversation([
      row("user", "u1", "q"),
      row("assistant", "a1", "first"),
      row("tool", "t1", '{"command":"a"}', "Bash"),
      row("tool", "t2", '{"command":"b"}', "Bash"),
      row("assistant", "a2", "second"),
    ])
    expect(turns[0].nodes[0].tools.map((t) => t.uuid)).toEqual(["t1", "t2"])
    expect(turns[0].nodes[1].tools).toEqual([])
  })
})

describe("userTurnCount / toolTurnCount（活动卡的两个轮次读端）", () => {
  it("轮数 = 用户轮（number ≥ 1），prelude 不计入", () => {
    const turns = groupConversation([
      row("system", "sys", "context"),
      row("user", "u1", "q1"),
      row("assistant", "a1", "r1"),
      row("user", "u2", "q2"),
      row("assistant", "a2", "r2"),
    ])
    expect(userTurnCount(turns)).toBe(2)
  })

  it("含工具轮数 = 任一节点挂工具块的轮数（含 loose 组）", () => {
    const turns = groupConversation([
      row("user", "u1", "q1"),
      row("assistant", "a1", "r1"), // 无工具轮
      row("user", "u2", "q2"),
      row("assistant", "a2", "r2"),
      row("tool", "t1", '{"command":"ls"}', "Bash"), // 挂到 a2 → 含工具轮
      row("user", "u3", "q3"),
      row("tool", "t2", '{"command":"pwd"}', "Bash"), // 无助手行 → loose 组也算含工具轮
    ])
    expect(toolTurnCount(turns)).toBe(2)
  })

  it("一次切片喂两个读端：轮数与含工具轮数互不污染", () => {
    const turns = groupConversation([
      row("user", "u1", "q"),
      row("assistant", "a1", "r"),
      row("tool", "t1", "{}", "Bash"),
    ])
    expect(userTurnCount(turns)).toBe(1)
    expect(toolTurnCount(turns)).toBe(1)
  })

  it("空转录（无任何轮）→ 两个计数都为 0", () => {
    expect(userTurnCount(groupConversation([]))).toBe(0)
    expect(toolTurnCount(groupConversation([]))).toBe(0)
  })
})

describe("conversationLayout", () => {
  it("maps every uuid to its turn, attached tools to their owner, loose tools to their group", () => {
    const messages = [
      row("system", "sys", "ctx"),
      row("user", "u1", "q1"),
      row("assistant", "a1", "text"),
      row("tool", "t1", '{"command":"ls"}', "Bash"),
      row("user", "u2", "q2"),
      row("tool", "t2", '{"command":"pwd"}', "Bash"),
    ]
    const layout = conversationLayout(groupConversation(messages))
    expect(layout.turnOf.get("sys")).toBe(0)
    expect(layout.turnOf.get("u1")).toBe(1)
    expect(layout.turnOf.get("a1")).toBe(1)
    expect(layout.turnOf.get("t1")).toBe(1)
    expect(layout.turnOf.get("u2")).toBe(2)
    expect(layout.attachedTo.get("t1")).toBe("a1")
    expect(layout.attachedTo.has("t2")).toBe(false)
    expect(layout.looseGroup.get("t2")?.map((t) => t.uuid)).toEqual(["t2"])
  })
})

describe("toolSummary", () => {
  it("prefers the common argument keys, in order", () => {
    expect(toolSummary('{"file_path":"D:\\\\x\\\\main.rs","offset":3}')).toBe(
      "D:\\x\\main.rs",
    )
    expect(toolSummary('{"pattern":"fn main","path":"src"}')).toBe("fn main")
  })

  it("falls through to the first string value, then nothing for object-only bodies", () => {
    expect(toolSummary('{"todo":"write tests","count":2}')).toBe("write tests")
    expect(toolSummary('{"items":[1,2],"done":true}')).toBe("")
  })

  it("keeps one line and caps the length", () => {
    const multi = `{"command":"git status\\nsecond line"}`
    expect(toolSummary(multi)).toBe("git status")
    const long = `{"command":"${"a".repeat(120)}"}`
    expect(toolSummary(long)).toBe(`${"a".repeat(95)}…`)
  })

  it("degrades plain-text bodies (non-claude sources) to their first line", () => {
    expect(toolSummary("plain tool output\nsecond line")).toBe(
      "plain tool output",
    )
    expect(toolSummary("   ")).toBe("")
    expect(toolSummary("123")).toBe("123")
  })
})

describe("ownsFlowRow", () => {
  const index = (rows: SessionMessage[]) =>
    Object.fromEntries(rows.map((r) => [r.uuid, r]))

  it("tool rows absorbed by an assistant card own no row of their own", () => {
    const rows = [
      row("user", "u1", "edit the file"),
      row("assistant", "a1", "on it"),
      row("tool", "t1", '{"file_path":"x"}', "Edit"),
    ]
    const layout = conversationLayout(groupConversation(rows))
    const m = index(rows)
    expect(ownsFlowRow(m.t1, layout)).toBe(false)
    expect(ownsFlowRow(m.a1, layout)).toBe(true)
  })

  it("a loose tool run renders once at its first member; later members do not", () => {
    // No assistant text in the turn → tools stay standalone as a loose group.
    const rows = [
      row("user", "u2", "run something"),
      row("tool", "t2", "{}", "Bash"),
      row("tool", "t3", "{}", "Read"),
    ]
    const layout = conversationLayout(groupConversation(rows))
    const m = index(rows)
    expect(ownsFlowRow(m.t2, layout)).toBe(true)
    expect(ownsFlowRow(m.t3, layout)).toBe(false)
  })

  it("every non-tool role owns its row", () => {
    const rows = [
      row("system", "sys", "context"),
      row("user", "u3", "q"),
      row("assistant", "a3", "r"),
    ]
    const layout = conversationLayout(groupConversation(rows))
    const m = index(rows)
    expect(ownsFlowRow(m.sys, layout)).toBe(true)
    expect(ownsFlowRow(m.u3, layout)).toBe(true)
    expect(ownsFlowRow(m.a3, layout)).toBe(true)
  })
})
