// Tests for the sessions browser derivations (architecture.md: "测试必须跑生产
// 路径"). Every function in ./derive is pure, so these are table-driven unit
// cases run in vitest's node-only environment (no DOM — see vitest.config.ts).

import { describe, expect, it } from "vitest"

import type {
  SessionGroup,
  SessionMessage,
  SessionRow,
} from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  applyGroupOrder,
  canCreateSyncedGroup,
  collapseAllMessages,
  effectiveFavorite,
  expandAllMessages,
  favKey,
  firstLine,
  isAllCollapsed,
  modelsUsed,
  neighborNav,
  nextFavValue,
  reorderGroupIds,
  sessionSpan,
  sessionTabFilter,
  transcriptMatches,
  tryFormatJson,
  UNGROUPED,
  ungroupedCount,
  withFavOverride,
  withoutFavOverride,
} from "./derive"

/** Minimal factory: spread overrides over a zero-valued SessionRow so each case
 *  only spells out the fields that matter. Mirrors the backend's SessionRow
 *  shape (snake_case, all fields present). */
function row(
  overrides: Partial<SessionRow> & Pick<SessionRow, "id">,
): SessionRow {
  return {
    device_id: "dev-self",
    source: "claude_code",
    project_dir: "",
    title: "",
    agent_type: "",
    favorited: false,
    local_group_id: "",
    synced_group_id: "",
    started_at: "",
    last_active_at: "",
    request_count: 0,
    total_tokens: 0,
    total_cost_usd: null,
    ...overrides,
  }
}

// ---------------------------------------------------------------- filters ----

describe("sessionTabFilter", () => {
  it("local tab scopes to this device with no favorite constraint", () => {
    expect(sessionTabFilter("local", "dev-self")).toEqual({
      device_scope: "dev-self",
      source: null,
      favorited: null,
      local_group_id: null,
      synced_group_id: null,
      from_ts: null,
      to_ts: null,
      model: null,
      search: null,
    })
  })

  it("favorites tab scopes to favorited across all devices", () => {
    expect(sessionTabFilter("favorites", "dev-self")).toEqual({
      device_scope: null,
      source: null,
      favorited: true,
      local_group_id: null,
      synced_group_id: null,
      from_ts: null,
      to_ts: null,
      model: null,
      search: null,
    })
  })

  it("local and favorites filters are distinct (no overlap in scopes)", () => {
    const local = sessionTabFilter("local", "dev-self")
    const fav = sessionTabFilter("favorites", "dev-self")
    expect(local).not.toEqual(fav)
  })

  it("default source is null (all sources) for both tabs", () => {
    expect(sessionTabFilter("local", "dev-self").source).toBeNull()
    expect(sessionTabFilter("favorites", "dev-self").source).toBeNull()
  })

  it("source arg narrows the tab slice on both tabs", () => {
    expect(
      sessionTabFilter("local", "dev-self", { source: "claude_code" }),
    ).toEqual({
      device_scope: "dev-self",
      source: "claude_code",
      favorited: null,
      local_group_id: null,
      synced_group_id: null,
      from_ts: null,
      to_ts: null,
      model: null,
      search: null,
    })
    expect(
      sessionTabFilter("favorites", "dev-self", { source: "codex_cli" }),
    ).toEqual({
      device_scope: null,
      source: "codex_cli",
      favorited: true,
      local_group_id: null,
      synced_group_id: null,
      from_ts: null,
      to_ts: null,
      model: null,
      search: null,
    })
  })

  it("an empty source string is treated as null (no constraint)", () => {
    expect(
      sessionTabFilter("local", "dev-self", { source: "" }).source,
    ).toBeNull()
  })

  it("time range narrows both tabs (last_active_at bounds)", () => {
    expect(
      sessionTabFilter("local", "dev-self", {
        fromTs: "2026-08-01T00:00:00.000Z",
        toTs: "2026-08-31T23:59:59.999Z",
      }).from_ts,
    ).toBe("2026-08-01T00:00:00.000Z")
    expect(
      sessionTabFilter("favorites", "dev-self", {
        fromTs: "2026-08-01T00:00:00.000Z",
      }).to_ts,
    ).toBeNull()
  })

  it("deviceScope narrows favorites but is ignored on local", () => {
    // Local tab always uses selfDeviceId — deviceScope is ignored.
    expect(
      sessionTabFilter("local", "dev-self", { deviceScope: "dev-peer" })
        .device_scope,
    ).toBe("dev-self")
    // Favorites tab honors deviceScope.
    expect(
      sessionTabFilter("favorites", "dev-self", { deviceScope: "dev-peer" })
        .device_scope,
    ).toBe("dev-peer")
  })

  it("model arg narrows both tabs; empty string becomes null", () => {
    expect(
      sessionTabFilter("local", "dev-self", { model: "glm-5.2" }).model,
    ).toBe("glm-5.2")
    expect(
      sessionTabFilter("favorites", "dev-self", { model: "glm-5.2" }).model,
    ).toBe("glm-5.2")
    expect(
      sessionTabFilter("local", "dev-self", { model: "" }).model,
    ).toBeNull()
  })

  it("search flows into the backend filter (paged search is backend-side)", () => {
    expect(
      sessionTabFilter("local", "dev-self", { search: "hello" }).search,
    ).toBe("hello")
    expect(
      sessionTabFilter("favorites", "dev-self", { search: "hello" }).search,
    ).toBe("hello")
    // Empty / null search = no constraint.
    expect(
      sessionTabFilter("local", "dev-self", { search: "" }).search,
    ).toBeNull()
    expect(sessionTabFilter("local", "dev-self").search).toBeNull()
  })

  it("groupId narrows the track's group column (null = all)", () => {
    // Local tab → local_group_id; a real id narrows to that group.
    expect(
      sessionTabFilter("local", "dev-self", {}, "lg1").local_group_id,
    ).toBe("lg1")
    // Favorites tab → synced_group_id (track-scoped, disjoint spaces).
    expect(
      sessionTabFilter("favorites", "dev-self", {}, "sg1").synced_group_id,
    ).toBe("sg1")
    // The other track's column stays null.
    expect(
      sessionTabFilter("favorites", "dev-self", {}, "sg1").local_group_id,
    ).toBeNull()
  })

  it("UNGROUPED maps to an empty group id (matches ungrouped rows)", () => {
    expect(
      sessionTabFilter("local", "dev-self", {}, UNGROUPED).local_group_id,
    ).toBe("")
    expect(
      sessionTabFilter("favorites", "dev-self", {}, UNGROUPED).synced_group_id,
    ).toBe("")
  })

  it("ALL_GROUPS and null groupId both mean no group constraint", () => {
    expect(
      sessionTabFilter("local", "dev-self", {}, ALL_GROUPS).local_group_id,
    ).toBeNull()
    expect(sessionTabFilter("local", "dev-self").local_group_id).toBeNull()
  })
})

// --------------------------------------------------------------- counts ----

describe("ungroupedCount", () => {
  it("total minus known-group buckets", () => {
    expect(
      ungroupedCount(
        {
          total: 10,
          groups: [
            { group_id: "lg1", count: 4 },
            { group_id: "lg2", count: 3 },
            { group_id: "", count: 3 },
          ],
        },
        new Set(["lg1", "lg2"]),
      ),
    ).toBe(3)
  })

  it("a stale group id (group since deleted) counts as ungrouped", () => {
    expect(
      ungroupedCount(
        {
          total: 7,
          groups: [
            { group_id: "lg1", count: 4 },
            { group_id: "ghost", count: 2 },
            { group_id: "", count: 1 },
          ],
        },
        new Set(["lg1"]),
      ),
    ).toBe(3)
  })

  it("a bucket id outside this track's groups counts as ungrouped", () => {
    // The synced-track bucket list contains only synced ids; a local-track
    // id would never appear, but the count stays correct if it did.
    expect(
      ungroupedCount(
        { total: 5, groups: [{ group_id: "sg1", count: 5 }] },
        new Set(["lg1"]),
      ),
    ).toBe(5)
  })

  it("no groups known → everything is ungrouped", () => {
    expect(
      ungroupedCount(
        { total: 3, groups: [{ group_id: "", count: 3 }] },
        new Set(),
      ),
    ).toBe(3)
  })

  it("all grouped → zero ungrouped", () => {
    expect(
      ungroupedCount(
        { total: 5, groups: [{ group_id: "lg1", count: 5 }] },
        new Set(["lg1"]),
      ),
    ).toBe(0)
  })
})

// --------------------------------------------------------------- favorites --

describe("favKey", () => {
  it("joins device_id and id with a slash", () => {
    expect(favKey({ device_id: "dev-a", id: "s1" })).toBe("dev-a/s1")
  })
})

describe("effectiveFavorite", () => {
  const s = row({ id: "s1", device_id: "dev-a", favorited: false })

  it("falls back to the row's favorited when no override is pending", () => {
    expect(effectiveFavorite(s, {})).toBe(false)
  })

  it("an override wins over the row's favorited", () => {
    expect(effectiveFavorite(s, { "dev-a/s1": true })).toBe(true)
  })

  it("the override is per-session (a same-id row on another device is unaffected)", () => {
    expect(effectiveFavorite(s, { "dev-b/s1": true })).toBe(false)
  })
})

describe("nextFavValue", () => {
  it("negates the effective state", () => {
    const s = row({ id: "s1", device_id: "dev-a", favorited: false })
    expect(nextFavValue(s, {})).toBe(true)
    expect(nextFavValue(s, { "dev-a/s1": true })).toBe(false)
  })
})

describe("withFavOverride / withoutFavOverride", () => {
  const s = row({ id: "s1", device_id: "dev-a" })

  it("withFavOverride stamps a value without mutating the input", () => {
    const prev = { "dev-b/s2": true }
    const next = withFavOverride(prev, s, true)
    expect(next).toEqual({ "dev-b/s2": true, "dev-a/s1": true })
    expect(prev).toEqual({ "dev-b/s2": true })
  })

  it("withoutFavOverride drops only the toggled key (rollback)", () => {
    const prev = { "dev-a/s1": true, "dev-b/s2": true }
    const next = withoutFavOverride(prev, s)
    expect(next).toEqual({ "dev-b/s2": true })
    expect(prev).toEqual({ "dev-a/s1": true, "dev-b/s2": true })
  })
})

describe("canCreateSyncedGroup", () => {
  it("local track is always allowed", () => {
    expect(canCreateSyncedGroup("local", false)).toBe(true)
    expect(canCreateSyncedGroup("local", true)).toBe(true)
  })

  it("synced track needs a bound repo", () => {
    expect(canCreateSyncedGroup("synced", false)).toBe(false)
    expect(canCreateSyncedGroup("synced", true)).toBe(true)
  })
})

describe("reorderGroupIds", () => {
  it("moves a group down (drag to a later slot)", () => {
    expect(reorderGroupIds(["a", "b", "c"], "a", "c")).toEqual(["b", "c", "a"])
  })

  it("moves a group up (drag to an earlier slot)", () => {
    expect(reorderGroupIds(["a", "b", "c"], "c", "a")).toEqual(["c", "a", "b"])
  })

  it("returns null when the drag lands where it started", () => {
    expect(reorderGroupIds(["a", "b", "c"], "a", "a")).toBeNull()
  })

  it("returns null on unknown ids (defensive)", () => {
    expect(reorderGroupIds(["a", "b"], "zz", "b")).toBeNull()
    expect(reorderGroupIds(["a", "b"], "a", "zz")).toBeNull()
  })
})

describe("applyGroupOrder", () => {
  const groups = (ids: string[]): SessionGroup[] =>
    ids.map((id) => ({
      id,
      name: id,
      kind: "local",
      device_id: "",
    }))

  it("null override keeps the natural order", () => {
    const g = groups(["a", "b", "c"])
    expect(applyGroupOrder(g, null)).toEqual(g)
  })

  it("applies the dragged order", () => {
    expect(
      applyGroupOrder(groups(["a", "b", "c"]), ["c", "a", "b"]).map(
        (g) => g.id,
      ),
    ).toEqual(["c", "a", "b"])
  })

  it("groups missing from the override sort last, never dropped", () => {
    // "c" was deleted mid-flight — it still renders, at the end.
    const out = applyGroupOrder(groups(["a", "b", "c"]), ["b", "a"])
    expect(out.map((g) => g.id)).toEqual(["b", "a", "c"])
  })
})

// --------------------------------------------------------------- detail -----

describe("sessionSpan", () => {
  const cases: Array<{
    name: string
    ms: number | null | undefined
    want: unknown
  }> = [
    {
      name: "under a minute rounds down to 0 minutes",
      ms: 59_999,
      want: { days: 0, hours: 0, minutes: 0 },
    },
    {
      name: "a few minutes",
      ms: 5 * 60_000 + 30_000,
      want: { days: 0, hours: 0, minutes: 5 },
    },
    {
      name: "hours and minutes",
      ms: 2 * 3_600_000 + 5 * 60_000,
      want: { days: 0, hours: 2, minutes: 5 },
    },
    {
      name: "days and hours",
      ms: 3 * 86_400_000 + 7 * 3_600_000,
      want: { days: 3, hours: 7, minutes: 0 },
    },
    { name: "null is null (no duration)", ms: null, want: null },
    { name: "zero is null", ms: 0, want: null },
    { name: "negative is null (times crossed)", ms: -1000, want: null },
    { name: "NaN is null", ms: NaN, want: null },
  ]
  for (const c of cases) {
    it(c.name, () => {
      expect(sessionSpan(c.ms)).toEqual(c.want)
    })
  }
})

describe("modelsUsed", () => {
  const msg = (overrides: Partial<SessionMessage>): SessionMessage => ({
    uuid: "u",
    session_id: "s",
    role: "assistant",
    ts: "",
    content: "",
    ...overrides,
  })

  it("collects distinct models in first-use order", () => {
    const models = modelsUsed([
      msg({ uuid: "a", model: "claude-fable-5" }),
      msg({ uuid: "b", role: "user", model: null }),
      msg({ uuid: "c", model: "claude-sonnet-5" }),
      msg({ uuid: "d", model: "claude-fable-5" }),
    ])
    expect(models).toEqual(["claude-fable-5", "claude-sonnet-5"])
  })

  it("ignores empty / whitespace models and nulls", () => {
    expect(
      modelsUsed([
        msg({ uuid: "a", model: null }),
        msg({ uuid: "b", model: "  " }),
        msg({ uuid: "c", model: "glm-5.2" }),
      ]),
    ).toEqual(["glm-5.2"])
  })

  it("empty transcript yields an empty list", () => {
    expect(modelsUsed([])).toEqual([])
  })
})

// ------------------------------------------------------------- transcript --

describe("neighborNav", () => {
  const rows = (n: number, start = 0): SessionRow[] =>
    Array.from({ length: n }, (_, i) =>
      row({ id: `s${start + i}`, title: `s${start + i}` }),
    )
  // previewKey arrives in favKey form ("device_id/id") — match how the hook
  // derives it from the preview row.
  const key = (id: string) => favKey({ device_id: "dev-self", id })

  it("rows mid-page navigate both ways", () => {
    expect(neighborNav(rows(5), key("s2"), 0, 20, 5)).toEqual({
      canPrev: true,
      canNext: true,
    })
  })

  it("first row of the first page has no prev", () => {
    expect(neighborNav(rows(5), key("s0"), 0, 20, 5)).toEqual({
      canPrev: false,
      canNext: true,
    })
  })

  it("last row of the last page has no next", () => {
    expect(neighborNav(rows(5), key("s4"), 0, 20, 5)).toEqual({
      canPrev: true,
      canNext: false,
    })
  })

  it("a page-edge row can page forward when more pages exist", () => {
    // Last row of page 2 (rows s20-39 of 45, offset 20): next pages into page 3.
    expect(neighborNav(rows(20, 20), key("s39"), 20, 20, 45)).toEqual({
      canPrev: true,
      canNext: true,
    })
  })

  it("a page-edge row can page backward when not on the first page", () => {
    // First row of page 2 (offset 20): prev pages back into page 1.
    expect(neighborNav(rows(20, 20), key("s20"), 20, 20, 45)).toEqual({
      canPrev: true,
      canNext: true,
    })
  })

  it("an off-page key (filter changed mid-session) disables both", () => {
    expect(neighborNav(rows(5), key("s99"), 0, 20, 5)).toEqual({
      canPrev: false,
      canNext: false,
    })
    expect(neighborNav(rows(5), null, 0, 20, 5)).toEqual({
      canPrev: false,
      canNext: false,
    })
  })
})

describe("collapseAllMessages / expandAllMessages / isAllCollapsed", () => {
  const msgs = (...roles: SessionMessage["role"][]): SessionMessage[] =>
    roles.map((role, i) => ({
      uuid: `u${i}`,
      role,
      content: "",
    })) as SessionMessage[]

  it("collapseAll puts every non-tool row in the set", () => {
    expect(
      collapseAllMessages(msgs("user", "assistant", "tool", "system")),
    ).toEqual(new Set(["u0", "u1", "u3"]))
  })

  it("expandAll puts every tool row in the set", () => {
    expect(
      expandAllMessages(msgs("user", "assistant", "tool", "system")),
    ).toEqual(new Set(["u2"]))
  })

  it("isAllCollapsed is true only when every non-tool row is in the set", () => {
    const all = msgs("user", "assistant", "tool", "system")
    expect(isAllCollapsed(all, new Set(["u0", "u1", "u3"]))).toBe(true)
    expect(isAllCollapsed(all, new Set(["u0", "u3"]))).toBe(false)
  })

  it("isAllCollapsed is false on a tool-only or empty transcript", () => {
    expect(isAllCollapsed(msgs("tool"), new Set())).toBe(false)
    expect(isAllCollapsed([], new Set())).toBe(false)
  })

  it("bulk sets round-trip through the row's own isOpen rule", () => {
    // Simulate the detail view's isOpen: non-tool open = not-in-set, tool
    // open = in-set. After collapseAll no row is open; after expandAll every
    // row is open.
    const all = msgs("user", "tool", "assistant")
    const isOpen = (m: SessionMessage, set: Set<string>) =>
      m.role === "tool" ? set.has(m.uuid) : !set.has(m.uuid)
    const collapsed = collapseAllMessages(all)
    expect(all.every((m) => !isOpen(m, collapsed))).toBe(true)
    const expanded = expandAllMessages(all)
    expect(all.every((m) => isOpen(m, expanded))).toBe(true)
  })
})

describe("firstLine", () => {
  it("returns the first line of multiline text", () => {
    expect(firstLine("hello\nworld\n")).toBe("hello")
  })

  it("single-line text returns itself", () => {
    expect(firstLine("solo")).toBe("solo")
  })

  it("empty text yields an empty string", () => {
    expect(firstLine("")).toBe("")
  })
})

describe("tryFormatJson", () => {
  it("pretty-prints an object with 2-space indent", () => {
    expect(tryFormatJson('{"a":1,"b":[2,3]}')).toBe(
      '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ]\n}',
    )
  })

  it("pretty-prints an array", () => {
    expect(tryFormatJson('[1,"x"]')).toBe('[\n  1,\n  "x"\n]')
  })

  it("rejects plain text", () => {
    expect(tryFormatJson("not json at all")).toBeNull()
  })

  it("rejects malformed json", () => {
    expect(tryFormatJson('{"a":')).toBeNull()
  })

  it("rejects scalar json (a bare string or number formats to nothing)", () => {
    expect(tryFormatJson('"just a string"')).toBeNull()
    expect(tryFormatJson("42")).toBeNull()
  })
})

describe("transcriptMatches", () => {
  let seq = 0
  const msg = (
    content: string,
    ts = "2026-08-12T10:00:00Z",
  ): SessionMessage => {
    seq += 1
    return {
      uuid: `u${seq}`,
      role: "assistant",
      content,
      ts,
      session_id: "s",
    } as SessionMessage
  }

  it("empty / whitespace query → no hits", () => {
    expect(transcriptMatches([msg("hello world")], "")).toEqual([])
    expect(transcriptMatches([msg("hello world")], "   ")).toEqual([])
  })

  it("no match → no hits", () => {
    expect(transcriptMatches([msg("hello world")], "nope")).toEqual([])
  })

  it("matches are case-insensitive and keep transcript order", () => {
    const ms = [
      msg("Fix the BUG here"),
      msg("another one"),
      msg("no bug at all"),
    ]
    expect(transcriptMatches(ms, "bug").map((h) => h.message)).toEqual([
      ms[0],
      ms[2],
    ])
  })

  it("snippet keeps the hit intact for the renderer to highlight", () => {
    const [hit] = transcriptMatches(
      [msg("prefix 1234567890 bug 1234567890 suffix")],
      "bug",
    )
    expect(hit.snippet).toContain("bug")
  })

  it("snippet ellipsizes both edges when the hit sits mid-text", () => {
    const text = `${"x".repeat(100)} needle ${"y".repeat(100)}`
    const [hit] = transcriptMatches([msg(text)], "needle")
    expect(hit.snippet.startsWith("…")).toBe(true)
    expect(hit.snippet.endsWith("…")).toBe(true)
    // RADIUS (28) both sides + the 6-char hit + 2 ellipses.
    expect(hit.snippet).toHaveLength(28 + 6 + 28 + 2)
  })

  it("snippet at the start of the text has no leading ellipsis", () => {
    const text = `needle ${"y".repeat(100)}`
    const [hit] = transcriptMatches([msg(text)], "needle")
    expect(hit.snippet.startsWith("needle")).toBe(true)
    expect(hit.snippet.endsWith("…")).toBe(true)
  })

  it("snippet at the end of the text has no trailing ellipsis", () => {
    const text = `${"x".repeat(100)} needle`
    const [hit] = transcriptMatches([msg(text)], "needle")
    expect(hit.snippet.endsWith("needle")).toBe(true)
  })
})
