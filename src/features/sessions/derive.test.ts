// Tests for the sessions browser derivations (architecture.md: "测试必须跑生产
// 路径"). Every function in ./derive is pure, so these are table-driven unit
// cases run in vitest's node-only environment (no DOM — see vitest.config.ts).

import { describe, expect, it } from "vitest"

import {
  DEFAULT_FILTER,
  FILTER_DIMENSIONS,
} from "@/app/store/slices/filterSlice"
import type {
  SessionGroup,
  SessionMessage,
  SessionRow,
  SessionStatsRow,
} from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  aggregateStats,
  applyGroupOrder,
  canCreateSyncedGroup,
  collapseAllMessages,
  effectiveFavorite,
  expandAllMessages,
  favKey,
  firstLine,
  groupedRows,
  identityOfProjectFilter,
  isAllCollapsed,
  isRowOpen,
  modelsUsed,
  neighborNav,
  nextFavValue,
  projectFilterOfIdentity,
  projectNodes,
  reorderGroupIds,
  roleDefaultsCollapsed,
  type SessionScopeSpec,
  sessionSpan,
  sessionSpecId,
  sessionTabFilter,
  spanLabelKey,
  tokensHitRate,
  transcriptMatches,
  tryFormatJson,
  UNGROUPED,
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
      project: null,
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
      project: null,
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
      project: null,
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
      project: null,
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

describe("spanLabelKey", () => {
  it("days win → days+hours label", () => {
    expect(spanLabelKey({ days: 3, hours: 7, minutes: 0 })).toEqual({
      key: "sessions.span.daysHours",
      vars: { d: 3, h: 7 },
    })
  })
  it("hours + minutes → hoursMinutes; hours only → hours", () => {
    expect(spanLabelKey({ days: 0, hours: 2, minutes: 5 })).toEqual({
      key: "sessions.span.hoursMinutes",
      vars: { h: 2, m: 5 },
    })
    expect(spanLabelKey({ days: 0, hours: 2, minutes: 0 })).toEqual({
      key: "sessions.span.hours",
      vars: { h: 2 },
    })
  })
  it("minutes only → minutes label; null → null (caller renders the dash)", () => {
    expect(spanLabelKey({ days: 0, hours: 0, minutes: 5 })).toEqual({
      key: "sessions.span.minutes",
      vars: { m: 5 },
    })
    expect(spanLabelKey(null)).toBeNull()
  })
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

  it("bulk sets round-trip through the detail view's real isRowOpen", () => {
    // The detail view's per-row open state runs the same xor rule as the bulk
    // sets (single source in derive): after collapseAll no row is open, after
    // expandAll every row is open. Calls the production isRowOpen — no fork
    // (architecture.md: "测试必须跑生产路径").
    const all = msgs("user", "tool", "assistant")
    const collapsed = collapseAllMessages(all)
    expect(all.every((m) => !isRowOpen(m.uuid, m.role, collapsed))).toBe(true)
    const expanded = expandAllMessages(all)
    expect(all.every((m) => isRowOpen(m.uuid, m.role, expanded))).toBe(true)
  })

  it("read/write consistency: every role's default matches both ends", () => {
    // The xor rule has one source (roleDefaultsCollapsed). A third
    // default-collapsed role would have to change that predicate, and this
    // table forces the change there — not silently in two files.
    for (const role of ["user", "assistant", "tool", "system"] as const) {
      const m = msgs(role)[0]
      const collapsed = collapseAllMessages([m])
      const expanded = expandAllMessages([m])
      expect(
        isRowOpen(m.uuid, m.role, collapsed),
        `${role} row must be closed after collapseAll`,
      ).toBe(false)
      expect(
        isRowOpen(m.uuid, m.role, expanded),
        `${role} row must be open after expandAll`,
      ).toBe(true)
    }
  })
})

describe("roleDefaultsCollapsed", () => {
  it("only tool rows default collapsed", () => {
    expect(roleDefaultsCollapsed("tool")).toBe(true)
    expect(roleDefaultsCollapsed("user")).toBe(false)
    expect(roleDefaultsCollapsed("assistant")).toBe(false)
    expect(roleDefaultsCollapsed("system")).toBe(false)
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

// ------------------------------------------------------- cache keys --------

describe("sessionSpecId — cache-key dimension completeness", () => {
  const scope = (): SessionScopeSpec => ({
    filter: { ...DEFAULT_FILTER },
    tab: "local",
    selfDeviceId: "dev-self",
    selectedGroupId: "",
    search: null,
  })

  it("covers every FilterState dimension (no silent cache sharing)", () => {
    for (const dim of FILTER_DIMENSIONS) {
      const base = scope()
      const other: SessionScopeSpec = {
        ...base,
        filter: { ...base.filter, [dim]: "x" },
      }
      expect(
        sessionSpecId(other),
        `filter.${dim} must be part of sessionSpecId or two scopes share one cache entry`,
      ).not.toBe(sessionSpecId(base))
    }
  })

  it("covers the session-only dimensions", () => {
    const cases: Array<{ label: string; patch: Partial<SessionScopeSpec> }> = [
      { label: "tab", patch: { tab: "favorites" } },
      { label: "selfDeviceId", patch: { selfDeviceId: "dev-other" } },
      { label: "selectedGroupId", patch: { selectedGroupId: "g1" } },
      { label: "search", patch: { search: "query" } },
    ]
    for (const { label, patch } of cases) {
      const base = scope()
      const other: SessionScopeSpec = { ...base, ...patch }
      expect(
        sessionSpecId(other),
        `${label} must be part of sessionSpecId or two scopes share one cache entry`,
      ).not.toBe(sessionSpecId(base))
    }
  })
})

// --------------------------------------------- project dimension mapping ----

describe("projectFilterOfIdentity / identityOfProjectFilter (tree ↔ filter)", () => {
  // The tree buckets by session-side identity ("" = the no-launch-dir bucket);
  // the filter value space uses "" for "no constraint" and the unknown
  // sentinel (endpoint data, never a frontend literal) for that bucket.
  const SENTINEL = "__unknown_project__"

  it("a known identity passes through unchanged in both directions", () => {
    expect(projectFilterOfIdentity("/p/alpha", SENTINEL)).toBe("/p/alpha")
    expect(identityOfProjectFilter("/p/alpha", SENTINEL)).toBe("/p/alpha")
  })

  it('the empty tree bucket maps to the sentinel and back to ""', () => {
    expect(projectFilterOfIdentity("", SENTINEL)).toBe(SENTINEL)
    expect(identityOfProjectFilter(SENTINEL, SENTINEL)).toBe("")
  })

  it("an empty filter value maps to null (no selection)", () => {
    expect(identityOfProjectFilter("", SENTINEL)).toBeNull()
  })

  it("the empty bucket is not expressible without the sentinel value", () => {
    // No unknown usage ever seen → the endpoint never delivered the sentinel;
    // clicking the bucket degrades to "no constraint" instead of guessing.
    expect(projectFilterOfIdentity("", null)).toBe("")
  })

  it("a stale sentinel selection still maps back via the remembered value", () => {
    // unknownValue outlives the live option presence (useProjectCandidates
    // remembers it), so a window change cannot strand the mapping.
    expect(identityOfProjectFilter(SENTINEL, SENTINEL)).toBe("")
    // And with no remembered value the sentinel is indistinguishable from a
    // (pathological) plain identity — it names itself rather than mis-bucketing.
    expect(identityOfProjectFilter(SENTINEL, null)).toBe(SENTINEL)
  })
})

// ---------------------------------------------------- workbench stats ----

/** Minimal factory over a zero-valued SessionStatsRow (mirrors the backend
 *  shape; only the fields a case cares about get spelled out). */
function statsRow(
  overrides: Partial<SessionStatsRow> & Pick<SessionStatsRow, "id">,
): SessionStatsRow {
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
    message_count: 0,
    input_tokens: 0,
    output_tokens: 0,
    cache_creation_tokens: 0,
    cache_read_tokens: 0,
    cache_hit_rate: 0,
    total_cost_usd: 0,
    models: [],
    ...overrides,
  }
}

describe("tokensHitRate", () => {
  it("derives cache_read / (input + cache_creation + cache_read)", () => {
    expect(
      tokensHitRate({
        input: 30,
        output: 99,
        cache_creation: 10,
        cache_read: 60,
      }),
    ).toBeCloseTo(60 / 100)
  })

  it("null when the cacheable pool is empty (no usage)", () => {
    expect(
      tokensHitRate({ input: 0, output: 5, cache_creation: 0, cache_read: 0 }),
    ).toBeNull()
  })
})

describe("aggregateStats", () => {
  it("sums additively, merges models, and derives the hit rate from summed buckets", () => {
    const a = aggregateStats([
      statsRow({
        id: "s1",
        request_count: 3,
        message_count: 10,
        input_tokens: 100,
        output_tokens: 20,
        cache_creation_tokens: 10,
        cache_read_tokens: 70,
        total_cost_usd: 1.5,
        started_at: "2026-08-10T10:00:00Z",
        last_active_at: "2026-08-10T11:00:00Z",
        models: [
          { model: "glm-5.2", tokens: 180 },
          { model: "glm-5.2-air", tokens: 20 },
        ],
      }),
      statsRow({
        id: "s2",
        request_count: 1,
        message_count: 4,
        input_tokens: 50,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 50,
        total_cost_usd: 0.5,
        started_at: "2026-08-11T10:00:00Z",
        last_active_at: "2026-08-11T10:40:00Z",
        models: [{ model: "glm-5.2", tokens: 100 }],
      }),
    ])
    expect(a.sessions).toBe(2)
    expect(a.requests).toBe(4)
    expect(a.messages).toBe(14)
    expect(a.tokens).toEqual({
      input: 150,
      output: 20,
      cache_creation: 10,
      cache_read: 120,
    })
    expect(a.cost).toBeCloseTo(2.0)
    // NOT the mean of row rates — the summed-bucket formula only.
    expect(a.hitRate).toBeCloseTo(120 / 280)
    expect(a.models).toEqual([
      { model: "glm-5.2", tokens: 280, sessions: 2 },
      { model: "glm-5.2-air", tokens: 20, sessions: 1 },
    ])
    // 1h and 40m spans land in the 15–60m and 1–3h buckets.
    expect(a.durationBuckets).toEqual([0, 1, 1, 0])
    expect(a.lastActiveAt).toBe("2026-08-11T10:40:00Z")
  })

  it("skips invalid spans instead of bucketing garbage", () => {
    const a = aggregateStats([
      statsRow({
        id: "s1",
        // Negative span (last before start) and missing timestamps.
        started_at: "2026-08-12T10:00:00Z",
        last_active_at: "2026-08-11T10:00:00Z",
      }),
      statsRow({ id: "s2" }),
    ])
    expect(a.durationBuckets).toEqual([0, 0, 0, 0])
    // lastActiveAt tracks timestamps regardless of span validity — the
    // negative-span row still has a valid last_active_at.
    expect(a.lastActiveAt).toBe("2026-08-11T10:00:00Z")
  })
})

describe("projectNodes", () => {
  it("buckets by project identity and orders buckets by newest activity", () => {
    const nodes = projectNodes([
      statsRow({
        id: "a1",
        project_dir: "/p/alpha",
        last_active_at: "2026-08-10",
      }),
      statsRow({
        id: "b1",
        project_dir: "/p/beta",
        last_active_at: "2026-08-12",
      }),
      statsRow({
        id: "a2",
        project_dir: "/p/alpha",
        last_active_at: "2026-08-08",
        input_tokens: 30,
        output_tokens: 10,
      }),
    ])
    expect(nodes.map((n) => n.project)).toEqual(["/p/beta", "/p/alpha"])
    expect(nodes[1].sessions.map((s) => s.id)).toEqual(["a1", "a2"])
    expect(nodes[1].tokens).toBe(40)
    expect(nodes[1].lastActiveAt).toBe("2026-08-10")
  })
})

describe("groupedRows", () => {
  it("keeps known ids grouped; empty and stale ids fall to ungrouped", () => {
    const { grouped, ungrouped } = groupedRows(
      [
        { id: "s1", g: "g1" },
        { id: "s2", g: "" },
        { id: "s3", g: "gone" },
        { id: "s4", g: "g2" },
      ],
      (r) => r.g,
      new Set(["g1", "g2"]),
    )
    expect(grouped.get("g1")).toEqual([{ id: "s1", g: "g1" }])
    expect(grouped.get("g2")).toEqual([{ id: "s4", g: "g2" }])
    expect(ungrouped.map((r) => r.id)).toEqual(["s2", "s3"])
  })
})
