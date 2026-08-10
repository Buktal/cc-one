// Tests for the sessions browser derivations (architecture.md: "测试必须跑生产
// 路径"). Every function in ./derive is pure, so these are table-driven unit
// cases run in vitest's node-only environment (no DOM — see vitest.config.ts).

import { describe, expect, it } from "vitest"

import type { SessionGroup, SessionRow } from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  applyGroupOrder,
  canCreateSyncedGroup,
  effectiveFavorite,
  favKey,
  filterSessionsByQuery,
  firstLine,
  groupSessionsByGroup,
  nextFavValue,
  reorderGroupIds,
  selectSessions,
  sessionTabFilter,
  sortSessions,
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

function group(
  overrides: Partial<SessionGroup> & Pick<SessionGroup, "id" | "kind">,
): SessionGroup {
  return { name: overrides.id, device_id: "", ...overrides }
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
})

// ----------------------------------------------------------------- sort -----

describe("sortSessions", () => {
  const cases: Array<{ name: string; rows: SessionRow[]; want: string[] }> = [
    {
      name: "descending by last_active_at",
      rows: [
        row({ id: "old", last_active_at: "2026-08-01T00:00:00Z" }),
        row({ id: "new", last_active_at: "2026-08-04T00:00:00Z" }),
        row({ id: "mid", last_active_at: "2026-08-02T00:00:00Z" }),
      ],
      want: ["new", "mid", "old"],
    },
    {
      name: "missing timestamps sink to the end",
      rows: [
        row({ id: "notime", last_active_at: "" }),
        row({ id: "has", last_active_at: "2026-08-02T00:00:00Z" }),
      ],
      want: ["has", "notime"],
    },
    {
      name: "empty input",
      rows: [],
      want: [],
    },
  ]
  for (const c of cases) {
    it(c.name, () => {
      expect(sortSessions(c.rows).map((r) => r.id)).toEqual(c.want)
    })
  }

  it("does not mutate the input array", () => {
    const rows = [
      row({ id: "a", last_active_at: "2026-08-01T00:00:00Z" }),
      row({ id: "b", last_active_at: "2026-08-04T00:00:00Z" }),
    ]
    const snapshot = rows.map((r) => r.id)
    sortSessions(rows)
    expect(rows.map((r) => r.id)).toEqual(snapshot)
  })
})

// --------------------------------------------------------------- search -----

describe("filterSessionsByQuery", () => {
  const rows = [
    row({ id: "1", title: "Refactor parser", project_dir: "/a/b" }),
    row({ id: "2", title: "Hello", project_dir: "/projects/parser" }),
    row({ id: "3", title: "Unrelated", project_dir: "/x/y" }),
  ]

  const cases: Array<{ name: string; q: string; want: string[] }> = [
    { name: "matches title substring", q: "refactor", want: ["1"] },
    { name: "matches project substring", q: "parser", want: ["1", "2"] },
    { name: "case-insensitive", q: "HELLO", want: ["2"] },
    { name: "no match returns empty", q: "zzz", want: [] },
    { name: "empty query returns all unchanged", q: "", want: ["1", "2", "3"] },
    {
      name: "whitespace-only query returns all",
      q: "   ",
      want: ["1", "2", "3"],
    },
  ]
  for (const c of cases) {
    it(c.name, () => {
      expect(filterSessionsByQuery(rows, c.q).map((r) => r.id)).toEqual(c.want)
    })
  }
})

// -------------------------------------------------------------- groups ------

describe("groupSessionsByGroup", () => {
  const groups: SessionGroup[] = [
    group({ id: "lg1", kind: "local", name: "Local A" }),
    group({ id: "lg2", kind: "local", name: "Local B" }),
    group({ id: "sg1", kind: "synced", name: "Synced A" }),
  ]

  it("local track buckets by local_group_id and collects ungrouped", () => {
    const rows = [
      row({ id: "a", local_group_id: "lg1" }),
      row({ id: "b", local_group_id: "lg2" }),
      row({ id: "c", local_group_id: "" }),
      row({ id: "d", local_group_id: "lg1" }),
    ]
    const res = groupSessionsByGroup(rows, groups, "local")
    expect(res.groups.map((g) => g.group.id)).toEqual(["lg1", "lg2"])
    expect(res.groups[0].sessions.map((s) => s.id)).toEqual(["a", "d"])
    expect(res.groups[1].sessions.map((s) => s.id)).toEqual(["b"])
    expect(res.ungrouped.map((s) => s.id)).toEqual(["c"])
  })

  it("synced track buckets by synced_group_id (independent of local)", () => {
    const rows = [
      row({ id: "a", local_group_id: "lg1", synced_group_id: "sg1" }),
      row({ id: "b", synced_group_id: "" }),
    ]
    const res = groupSessionsByGroup(rows, groups, "synced")
    expect(res.groups.map((g) => g.group.id)).toEqual(["sg1"])
    expect(res.groups[0].sessions.map((s) => s.id)).toEqual(["a"])
    expect(res.ungrouped.map((s) => s.id)).toEqual(["b"])
  })

  it("a stale group id (group since deleted) falls into ungrouped, not dropped", () => {
    const rows = [row({ id: "a", local_group_id: "ghost" })]
    const res = groupSessionsByGroup(rows, groups, "local")
    expect(res.groups).toEqual([])
    expect(res.ungrouped.map((s) => s.id)).toEqual(["a"])
  })

  it("same session can sit in different groups across the two tracks", () => {
    const r = row({
      id: "x",
      local_group_id: "lg1",
      synced_group_id: "sg1",
    })
    const local = groupSessionsByGroup([r], groups, "local")
    const synced = groupSessionsByGroup([r], groups, "synced")
    expect(local.groups[0].group.id).toBe("lg1")
    expect(synced.groups[0].group.id).toBe("sg1")
  })

  it("empty groups are omitted from the result (sidebar shows them separately)", () => {
    const rows = [row({ id: "a", local_group_id: "lg1" })]
    const res = groupSessionsByGroup(rows, groups, "local")
    expect(res.groups.map((g) => g.group.id)).toEqual(["lg1"])
    // lg2 exists but has no sessions → not in result
  })

  it("preserves input order within each bucket (caller pre-sorts)", () => {
    // Pre-sorted descending by time.
    const rows = [
      row({
        id: "new",
        local_group_id: "lg1",
        last_active_at: "2026-08-04T00:00:00Z",
      }),
      row({
        id: "old",
        local_group_id: "lg1",
        last_active_at: "2026-08-01T00:00:00Z",
      }),
    ]
    const res = groupSessionsByGroup(rows, groups, "local")
    expect(res.groups[0].sessions.map((s) => s.id)).toEqual(["new", "old"])
  })

  it("empty input yields empty groups and ungrouped", () => {
    const res = groupSessionsByGroup([], groups, "local")
    expect(res.groups).toEqual([])
    expect(res.ungrouped).toEqual([])
  })
})

// -------------------------------------------------------- sidebar select ----

describe("selectSessions", () => {
  const groups: SessionGroup[] = [
    group({ id: "lg1", kind: "local", name: "Local A" }),
    group({ id: "lg2", kind: "local", name: "Local B" }),
  ]
  const rows = [
    row({ id: "a", local_group_id: "lg1" }),
    row({ id: "b", local_group_id: "lg2" }),
    row({ id: "c", local_group_id: "" }),
  ]
  const grouped = groupSessionsByGroup(rows, groups, "local")
  // allRows is the caller's sorted+filtered flat list.
  const allRows = rows

  it("ALL_GROUPS returns the full flat list (caller's sorted+filtered view)", () => {
    expect(
      selectSessions(allRows, grouped, ALL_GROUPS).map((r) => r.id),
    ).toEqual(["a", "b", "c"])
  })

  it("UNGROUPED returns only the ungrouped bucket", () => {
    expect(
      selectSessions(allRows, grouped, UNGROUPED).map((r) => r.id),
    ).toEqual(["c"])
  })

  it("a real group id returns that group's bucket", () => {
    expect(selectSessions(allRows, grouped, "lg1").map((r) => r.id)).toEqual([
      "a",
    ])
  })

  it("an unknown group id returns empty (defensive)", () => {
    expect(selectSessions(allRows, grouped, "nope")).toEqual([])
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

// ------------------------------------------------------------- transcript --

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
