// Conversation turn structure — the "which rows belong to which turn / which
// AI message owns which tool call" decision for the transcript view, as pure
// functions. Issue #86's settled shape: a turn = one user message through the
// message before the next user message (the same definition turn-nav.ts's
// turnAnchors owns — this module slices by those anchors, never by a second
// copy of the rule), and tool rows attach INSIDE the AI message card that
// issued them instead of interleaving as standalone rows.
//
// Why attachment runs on ORDER, not ids: the JSONL source splits one assistant
// API message into several single-block events (text and tool_use blocks never
// share an event; verified against real transcripts — 415 tool-only vs 0 mixed
// events in one session), and the collected SessionMessage carries no message
// id linking them. The only stable signal is sequence: within a turn, a run of
// tool rows attaches to the nearest PRECEDING assistant row, and a run that
// opens the turn (no assistant row above it yet) attaches to the turn's first
// assistant row below. A turn with no assistant rows at all keeps its tool
// rows as a standalone "tools" node rendered in place.

import type { SessionMessage } from "@/types/generated/bindings"

import { firstLine } from "./derive"
import { turnAnchors } from "./turn-nav"

/** One rendered block inside a turn: either a text row (user-adjacent system /
 *  assistant rows; assistant rows carry their attached tool calls), or a
 *  standalone run of tool rows that found no assistant row to attach to. */
export type ConversationNode =
  | { kind: "message"; message: SessionMessage; tools: SessionMessage[] }
  | { kind: "tools"; tools: SessionMessage[] }

/** One conversation turn: the user message that anchors it plus the rows up to
 *  (excluding) the next user message. `number` is the 1-based ordinal shown in
 *  the turn divider and shared with the turn-nav panel's numbering (both count
 *  user messages); 0 marks the prelude — rows before the first user message,
 *  which have no anchor and get no divider. */
export interface ConversationTurn {
  number: number
  user: SessionMessage | null
  nodes: ConversationNode[]
}

/**
 * Group a transcript into turns with tool calls attached to their AI message.
 * Pure: same input → same output. The turn boundaries come from turnAnchors
 * (single source of the "a user message starts a turn" rule); rows are visited
 * in order within each slice.
 */
export function groupConversation(
  messages: readonly SessionMessage[],
): ConversationTurn[] {
  const turns: ConversationTurn[] = []
  // Slice boundaries: the user-message indices turnAnchors reports. Prelude =
  // everything before the first anchor; a transcript that opens on a user
  // message has no prelude (skip the empty slice rather than emit a phantom
  // turn 0).
  const anchors = turnAnchors(messages)
  const bounds: Array<[start: number, end: number]> = []
  let prev = 0
  for (const a of anchors) {
    if (a.index > prev) bounds.push([prev, a.index])
    prev = a.index
  }
  if (prev < messages.length) bounds.push([prev, messages.length])
  // 1-based user-turn ordinal, shared with the turn-nav panel's numbering;
  // 0 is reserved for the prelude (rows before the first user message).
  let userTurnCount = 0
  for (const [start, end] of bounds) {
    const user =
      start < end && messages[start].role === "user" ? messages[start] : null
    const turn: ConversationTurn = {
      number: user ? ++userTurnCount : 0,
      user,
      nodes: [],
    }
    // Attachment state within the slice: `lastAssistant` receives tool runs
    // that follow it; `pending` holds a run that opened the slice (it attaches
    // to the first assistant row, or falls out as a standalone node at the end).
    let lastAssistant: Extract<ConversationNode, { kind: "message" }> | null =
      null
    let pending: SessionMessage[] = []
    for (let i = start + (user ? 1 : 0); i < end; i++) {
      const m = messages[i]
      if (m.role === "tool") {
        if (lastAssistant) lastAssistant.tools.push(m)
        else pending.push(m)
        continue
      }
      if (m.role === "assistant") {
        const node: Extract<ConversationNode, { kind: "message" }> = {
          kind: "message",
          message: m,
          tools: pending,
        }
        pending = []
        lastAssistant = node
        turn.nodes.push(node)
        continue
      }
      turn.nodes.push({ kind: "message", message: m, tools: [] })
    }
    if (pending.length > 0) turn.nodes.push({ kind: "tools", tools: pending })
    turns.push(turn)
  }
  return turns
}

/** The rendered-row lookup for one transcript: which turn each message sits in
 *  (for the divider), and for tool rows — the assistant row whose card renders
 *  them (attached), or the standalone-tools group that renders them (loose).
 *  Attached/loose tool rows render nothing on their own index inside the
 *  virtualized list (zero-height rows keep the message-index coordinate system
 *  that useTurnNav's rangeChanged / scrollToIndex speak in). */
export interface ConversationLayout {
  /** uuid → 1-based turn ordinal (0 for prelude rows) — drives the divider. */
  turnOf: Map<string, number>
  /** tool uuid → the assistant uuid whose card renders it. */
  attachedTo: Map<string, string>
  /** assistant uuid → the tool rows attached to its card. */
  toolsOf: Map<string, SessionMessage[]>
  /** tool uuid → the loose group that renders it (first row of the group
   *  renders the whole run; the rest render nothing on their own index). */
  looseGroup: Map<string, SessionMessage[]>
}

/** Build the row lookup for a grouped transcript. Pure. */
export function conversationLayout(
  turns: readonly ConversationTurn[],
): ConversationLayout {
  const turnOf = new Map<string, number>()
  const attachedTo = new Map<string, string>()
  const toolsOf = new Map<string, SessionMessage[]>()
  const looseGroup = new Map<string, SessionMessage[]>()
  for (const turn of turns) {
    if (turn.user) turnOf.set(turn.user.uuid, turn.number)
    for (const node of turn.nodes) {
      if (node.kind === "message") {
        turnOf.set(node.message.uuid, turn.number)
        for (const tool of node.tools) {
          turnOf.set(tool.uuid, turn.number)
          attachedTo.set(tool.uuid, node.message.uuid)
        }
        if (node.tools.length > 0) toolsOf.set(node.message.uuid, node.tools)
      } else {
        for (const tool of node.tools) {
          turnOf.set(tool.uuid, turn.number)
          looseGroup.set(tool.uuid, node.tools)
        }
      }
    }
  }
  return { turnOf, attachedTo, toolsOf, looseGroup }
}

/** Keys probed (in order) for the `Write · path`-style title summary — the
 *  argument field each common tool carries. Not exhaustive by design: a miss
 *  falls through to the first string-valued key, then to the raw text. */
const SUMMARY_KEYS = [
  "file_path",
  "notebook_path",
  "url",
  "pattern",
  "query",
  "command",
  "path",
  "prompt",
  "description",
] as const

/** Longest summary shown in a tool block's title row (chars) — the row is
 *  one line; the expanded body shows everything. */
const SUMMARY_MAX = 96

/**
 * The `· path`-style one-line summary of a tool call's arguments, for the tool
 * block's title row (`Write · src/lib/foo.ts`). Claude/codex rows carry the
 * tool_use input as a JSON object string; the other sources carry plain text,
 * so a non-JSON body falls back to its first line. Empty when there is nothing
 * worth showing (empty body / empty object) — the title then shows the tool
 * name alone.
 */
export function toolSummary(content: string): string {
  const text = content.trim()
  if (!text) return ""
  let value: unknown
  try {
    value = JSON.parse(text)
  } catch {
    return clip(firstLine(text))
  }
  if (typeof value !== "object" || value === null) return clip(firstLine(text))
  const record = value as Record<string, unknown>
  const hit =
    SUMMARY_KEYS.find((k) => typeof record[k] === "string") ??
    Object.keys(record).find((k) => typeof record[k] === "string")
  if (!hit) return ""
  return clip(firstLine(String(record[hit])))
}

/** One line, capped for the title row. */
function clip(line: string): string {
  return line.length > SUMMARY_MAX ? `${line.slice(0, SUMMARY_MAX - 1)}…` : line
}
