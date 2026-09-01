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
  SessionRow,
  SessionStatsRow,
} from "@/types/generated/bindings"
import {
  ALL_GROUPS,
  aggregateStats,
  applyGroupOrder,
  batchFailedCount,
  type ContainerSelection,
  canCreateSyncedGroup,
  containerLabel,
  containerScopeTag,
  containerStatsRows,
  effectiveFavorite,
  favKey,
  groupedRows,
  identityOfProjectFilter,
  neighborNav,
  nestSubagents,
  nextFavValue,
  parseTreeSelectValue,
  planNeighborStep,
  projectFilterOfIdentity,
  projectNodes,
  reorderGroupIds,
  resolveContainer,
  type SessionScopeSpec,
  sessionSpecId,
  sessionStartedAt,
  sessionTabFilter,
  settleNeighborStep,
  type TreeSelectAction,
  treeSelectValue,
  UNGROUPED,
  withCheckedToggle,
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
    parent_session_id: "",
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

// ------------------------------------------------------------ batch select --

describe("withCheckedToggle", () => {
  it("toggle：未勾选 → 加入（值保留行定位信息）", () => {
    const next = withCheckedToggle(new Map(), row({ id: "a" }))
    expect(next.get(favKey({ device_id: "dev-self", id: "a" }))).toEqual({
      id: "a",
      device_id: "dev-self",
    })
  })

  it("toggle：已勾选 → 移除", () => {
    const s = row({ id: "a" })
    const checked = withCheckedToggle(new Map(), s)
    expect(withCheckedToggle(checked, s).size).toBe(0)
  })

  it("返回新 Map，不动入参（React 更新纯度）", () => {
    const prev = new Map<string, { id: string; device_id: string }>()
    const next = withCheckedToggle(prev, row({ id: "a" }))
    expect(prev.size).toBe(0)
    expect(next.size).toBe(1)
    expect(prev).not.toBe(next)
  })
})

describe("batchFailedCount", () => {
  it("全成功 → 0（成功 toast 的判定）", () => {
    const results = [
      { status: "fulfilled" },
      { status: "fulfilled" },
    ] as PromiseSettledResult<unknown>[]
    expect(batchFailedCount(results)).toBe(0)
  })

  it("部分失败 → 失败数（部分失败警告的判定）", () => {
    const results = [
      { status: "fulfilled" },
      { status: "rejected", reason: new Error("x") },
      { status: "rejected", reason: new Error("y") },
    ] as PromiseSettledResult<unknown>[]
    expect(batchFailedCount(results)).toBe(2)
  })
})

// ------------------------------------------------------ session startedAt --

describe("sessionStartedAt", () => {
  const transcript = [{ ts: "2026-08-10T10:05:00Z" }]

  it("started_at 在 → 原样（不兜底）", () => {
    expect(
      sessionStartedAt({ started_at: "2026-08-10T10:00:00Z" }, transcript),
    ).toBe("2026-08-10T10:00:00Z")
  })

  it("started_at 缺采 → 首条消息时间兜底", () => {
    expect(sessionStartedAt({ started_at: "" }, transcript)).toBe(
      "2026-08-10T10:05:00Z",
    )
  })

  it("两者都缺 → null（调用方渲染占位）", () => {
    expect(sessionStartedAt({ started_at: "" }, [])).toBeNull()
    expect(sessionStartedAt({ started_at: "" }, [{ ts: "" }])).toBeNull()
  })
})

// --------------------------------------------------------------- detail -----
// 时长三件套（spanParts / spanLabelKey / spanMsOf）的用例随实现迁
// lib/format.test.ts（架构审查Ⅲ候选⑩）。

// --------------------------------------------------------- neighbor nav -----

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

// ---------------------------------- neighbor stepping (架构审查Ⅳ候选⑬) ----
// The page-edge walk the detail sheet's prev/next runs: plan the step, flip
// the page, settle the pending step when the new page's data lands. The three
// invariants — open only after the new page's data lands, drop the pending
// step when the user switches rows mid-load, clamp at the list ends — are
// pinned here on the exact functions the hook calls.

describe("planNeighborStep", () => {
  const page = (n: number, start = 0): SessionRow[] =>
    Array.from({ length: n }, (_, i) => row({ id: `s${start + i}` }))
  const key = (id: string) => favKey({ device_id: "dev-self", id })

  it("an adjacent row on the visible page opens directly", () => {
    const rows = page(5)
    expect(planNeighborStep(rows, key("s2"), 1, 0, 20, 5)).toEqual({
      kind: "in-page",
      target: rows[3],
    })
    expect(planNeighborStep(rows, key("s2"), -1, 0, 20, 5)).toEqual({
      kind: "in-page",
      target: rows[1],
    })
  })

  it("a page edge with a page beyond plans the flip and registers the pending step", () => {
    // Last row of page 1 (20 per page, 45 total): next flips to page 2, and
    // the pending step remembers the row it left from (the hijack guard).
    expect(planNeighborStep(page(20), key("s19"), 1, 0, 20, 45)).toEqual({
      kind: "page-edge",
      pending: { delta: 1, fromKey: key("s19") },
    })
    // First row of page 2: prev flips back to page 1 the same way.
    expect(planNeighborStep(page(20, 20), key("s20"), -1, 20, 20, 45)).toEqual({
      kind: "page-edge",
      pending: { delta: -1, fromKey: key("s20") },
    })
  })

  it("the ends clamp the plan (nowhere to step)", () => {
    const rows = page(5)
    // Last row of the last page: no next.
    expect(planNeighborStep(rows, key("s4"), 1, 0, 20, 5)).toEqual({
      kind: "stalled",
    })
    // First row of the first page: no prev.
    expect(planNeighborStep(rows, key("s0"), -1, 0, 20, 5)).toEqual({
      kind: "stalled",
    })
  })

  it("never plans past a boundary neighborNav disables (same source)", () => {
    const rows = page(5)
    for (const [k, offset, total] of [
      [key("s0"), 0, 5],
      [key("s4"), 0, 5],
      [key("s2"), 0, 5],
      [key("s0"), 20, 45],
      [key("s4"), 20, 45],
    ] as const) {
      const nav = neighborNav(rows, k, offset, 20, total)
      const fwd = planNeighborStep(rows, k, 1, offset, 20, total)
      const back = planNeighborStep(rows, k, -1, offset, 20, total)
      if (fwd.kind !== "stalled") expect(nav.canNext).toBe(true)
      if (back.kind !== "stalled") expect(nav.canPrev).toBe(true)
    }
  })

  it("a preview off the visible page (filter changed mid-session) stalls", () => {
    const rows = page(5)
    expect(planNeighborStep(rows, key("s99"), 1, 0, 20, 5)).toEqual({
      kind: "stalled",
    })
    expect(planNeighborStep(rows, null, -1, 0, 20, 5)).toEqual({
      kind: "stalled",
    })
  })
})

describe("settleNeighborStep", () => {
  const page = (n: number, start = 0): SessionRow[] =>
    Array.from({ length: n }, (_, i) => row({ id: `s${start + i}` }))
  const key = (id: string) => favKey({ device_id: "dev-self", id })

  it("opens only once the new page's data landed — next takes the first row, prev the last", () => {
    const page2 = page(20, 20)
    expect(
      settleNeighborStep({ delta: 1, fromKey: key("s19") }, key("s19"), page2),
    ).toEqual({ kind: "open", target: page2[0] })
    const page1 = page(20)
    expect(
      settleNeighborStep({ delta: -1, fromKey: key("s20") }, key("s20"), page1),
    ).toEqual({ kind: "open", target: page1[page1.length - 1] })
  })

  it("the flipped-to page still loading waits (pending stays registered)", () => {
    expect(
      settleNeighborStep({ delta: 1, fromKey: key("s19") }, key("s19"), []),
    ).toEqual({ kind: "wait" })
  })

  it("a mid-load row switch drops the pending step instead of hijacking the selection", () => {
    const page2 = page(20, 20)
    // The user clicked another row while the page loaded — fromKey no longer
    // matches, the step is discarded.
    expect(
      settleNeighborStep({ delta: 1, fromKey: key("s19") }, key("s25"), page2),
    ).toEqual({ kind: "drop" })
    // ...or the sheet was closed altogether.
    expect(
      settleNeighborStep({ delta: 1, fromKey: key("s19") }, null, page2),
    ).toEqual({ kind: "drop" })
  })

  it("a full page-edge walk: plan on page 1, settle opens page 2's first row", () => {
    const plan = planNeighborStep(page(20), key("s19"), 1, 0, 20, 45)
    if (plan.kind !== "page-edge") throw new Error("expected a page-edge plan")
    expect(settleNeighborStep(plan.pending, key("s19"), page(20, 20))).toEqual({
      kind: "open",
      target: row({ id: "s20" }),
    })
  })
})

// transcript 展示派生（行开合 xor 规则、批量收展、transcript 全文搜索、
// firstLine / tryFormatJson）的用例随实现迁 transcript.test.ts（架构审查Ⅳ
// 候选⑩）。

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

// ------------------------------------- container selection (架构审查候选⑤) --

describe("resolveContainer (priority ladder)", () => {
  const preview = row({
    id: "s1",
    title: "Refactor the parser",
    project_dir: "/p/alpha",
  })

  it("an open session beats every tree signal", () => {
    expect(resolveContainer(preview, "/p/alpha", "g1")).toEqual({
      kind: "session",
      id: "dev-self/s1",
      title: "Refactor the parser",
      dir: "/p/alpha",
    })
  })

  it('project beats group; the "" identity is a real selection, not "unset"', () => {
    expect(resolveContainer(null, "/p/alpha", "g1")).toEqual({
      kind: "project",
      id: "/p/alpha",
    })
    // "" = 无启动目录桶（哨兵映射后的 identity），以 != null 判定，仍压过分组。
    expect(resolveContainer(null, "", UNGROUPED)).toEqual({
      kind: "project",
      id: "",
    })
  })

  it("group sentinels resolve in order: ungrouped → all → concrete id", () => {
    expect(resolveContainer(null, null, UNGROUPED)).toEqual({
      kind: "ungrouped",
    })
    expect(resolveContainer(null, null, ALL_GROUPS)).toEqual({ kind: "all" })
    expect(resolveContainer(null, null, "g-rename")).toEqual({
      kind: "group",
      id: "g-rename",
    })
  })

  it("no signals at all resolves to all", () => {
    expect(resolveContainer(null, null, ALL_GROUPS)).toEqual({ kind: "all" })
  })
})

describe("containerScopeTag", () => {
  it("maps five kinds onto the rail's three tags; all shares the project cards", () => {
    const preview = row({ id: "s1", title: "T" })
    expect(containerScopeTag(resolveContainer(preview, null, ALL_GROUPS))).toBe(
      "session",
    )
    expect(containerScopeTag(resolveContainer(null, "/p/a", "g1"))).toBe(
      "project",
    )
    expect(containerScopeTag(resolveContainer(null, "", "g1"))).toBe("project")
    // 未选任何容器照旧共用项目卡组（无身份卡的全量聚合）——不加第四档。
    expect(containerScopeTag(resolveContainer(null, null, ALL_GROUPS))).toBe(
      "project",
    )
    expect(containerScopeTag(resolveContainer(null, null, UNGROUPED))).toBe(
      "group",
    )
    expect(containerScopeTag(resolveContainer(null, null, "g1"))).toBe("group")
  })
})

describe("containerLabel", () => {
  const groupNameOf = (id: string) => ({ g1: "Renamed group" })[id as "g1"]

  it("session uses its title, falling back to the untitled key", () => {
    expect(
      containerLabel(
        resolveContainer(row({ id: "s1", title: "T" }), null, ""),
        groupNameOf,
      ),
    ).toEqual({ text: "T" })
    expect(
      containerLabel(
        resolveContainer(row({ id: "s1", title: "" }), null, ""),
        groupNameOf,
      ),
    ).toEqual({ key: "sessions.untitled" })
  })

  it("project shows the basename; empty identity falls to noProject", () => {
    expect(
      containerLabel(resolveContainer(null, "/p/alpha", ""), groupNameOf),
    ).toEqual({ text: "alpha" })
    expect(containerLabel(resolveContainer(null, "", ""), groupNameOf)).toEqual(
      {
        key: "sessions.tree.noProject",
      },
    )
  })

  it("group resolves its name; a stale id falls back to tree.all", () => {
    expect(
      containerLabel(resolveContainer(null, null, "g1"), groupNameOf),
    ).toEqual({ text: "Renamed group" })
    expect(
      containerLabel(resolveContainer(null, null, "gone"), groupNameOf),
    ).toEqual({ key: "sessions.tree.all" })
  })

  it("sentinel kinds map to their keys", () => {
    expect(
      containerLabel(resolveContainer(null, null, UNGROUPED), groupNameOf),
    ).toEqual({
      key: "sessions.group.ungrouped",
    })
    expect(
      containerLabel(resolveContainer(null, null, ALL_GROUPS), groupNameOf),
    ).toEqual({
      key: "sessions.tree.all",
    })
  })
})

describe("treeSelectValue / parseTreeSelectValue round-trip", () => {
  it("every tree-expressible container survives encode → decode-as-action", () => {
    const cases: Array<[ContainerSelection, TreeSelectAction]> = [
      [{ kind: "all" }, { type: "all" }],
      [{ kind: "ungrouped" }, { type: "group", id: UNGROUPED }],
      [
        { kind: "group", id: "g1" },
        { type: "group", id: "g1" },
      ],
      [
        { kind: "project", id: "/p/alpha" },
        { type: "project", id: "/p/alpha" },
      ],
      // 未知项目桶编码为 "p:"——前缀下的空 identity 无歧义。
      [
        { kind: "project", id: "" },
        { type: "project", id: "" },
      ],
    ]
    for (const [container, action] of cases) {
      expect(parseTreeSelectValue(treeSelectValue(container))).toEqual(action)
    }
  })

  it('encodes the documented DSL shape ("p:<identity>" / "g:<id>" / "")', () => {
    expect(treeSelectValue({ kind: "all" })).toBe("")
    expect(treeSelectValue({ kind: "ungrouped" })).toBe(`g:${UNGROUPED}`)
    expect(treeSelectValue({ kind: "group", id: "g1" })).toBe("g:g1")
    expect(treeSelectValue({ kind: "project", id: "/p/alpha" })).toBe(
      "p:/p/alpha",
    )
    expect(parseTreeSelectValue("")).toEqual({ type: "all" })
    expect(parseTreeSelectValue("p:x")).toEqual({ type: "project", id: "x" })
    expect(parseTreeSelectValue("g:y")).toEqual({ type: "group", id: "y" })
  })
})

describe("containerStatsRows", () => {
  const alpha = statsRow({ id: "a", project_dir: "/p/alpha" })
  const beta = statsRow({ id: "b", project_dir: "/p/beta" })
  const universe = [alpha, beta]
  const buckets = {
    grouped: new Map([["g1", [alpha]]]),
    ungrouped: [beta],
  }

  it("project slices by identity (including the '' bucket)", () => {
    expect(
      containerStatsRows(
        { kind: "project", id: "/p/alpha" },
        universe,
        buckets,
      ),
    ).toEqual([alpha])
    expect(
      containerStatsRows({ kind: "project", id: "" }, universe, buckets),
    ).toEqual([])
  })

  it("group reads its bucket; an unknown id is empty, never the universe", () => {
    expect(
      containerStatsRows({ kind: "group", id: "g1" }, universe, buckets),
    ).toEqual([alpha])
    expect(
      containerStatsRows({ kind: "group", id: "gone" }, universe, buckets),
    ).toEqual([])
    expect(
      containerStatsRows({ kind: "ungrouped" }, universe, buckets),
    ).toEqual([beta])
  })

  it("all and session take the whole universe read", () => {
    expect(containerStatsRows({ kind: "all" }, universe, buckets)).toBe(
      universe,
    )
    expect(
      containerStatsRows(
        { kind: "session", id: "dev-self/s1", title: "T", dir: "" },
        universe,
        buckets,
      ),
    ).toBe(universe)
  })
})

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

describe("nestSubagents", () => {
  it("moves children directly under their in-slice parent, fetch order kept otherwise", () => {
    const parent1 = row({ id: "main-1", title: "P1" })
    const parent2 = row({ id: "main-2", title: "P2" })
    // Children arrive interleaved in the time-desc fetch order.
    const c2b = row({ id: "agent-c", parent_session_id: "main-2" })
    const c1 = row({ id: "agent-a", parent_session_id: "main-1" })
    const c2a = row({ id: "agent-b", parent_session_id: "main-2" })
    const { rows, nestedKeys } = nestSubagents([parent1, c2b, c1, parent2, c2a])
    expect(rows.map((r) => r.id)).toEqual([
      "main-1",
      "agent-a",
      "main-2",
      "agent-c",
      "agent-b",
    ])
    expect([...nestedKeys]).toEqual(
      expect.arrayContaining([
        "dev-self/agent-a",
        "dev-self/agent-b",
        "dev-self/agent-c",
      ]),
    )
    expect(nestedKeys.size).toBe(3)
  })

  it("keeps a child top-level when its parent is not in the slice", () => {
    const orphan = row({ id: "agent-x", parent_session_id: "main-elsewhere" })
    const other = row({ id: "main-1" })
    const { rows, nestedKeys } = nestSubagents([orphan, other])
    expect(rows.map((r) => r.id)).toEqual(["agent-x", "main-1"])
    expect(nestedKeys.size).toBe(0)
  })

  it("matches the parent on the composite key — a same-id row on another device is no parent", () => {
    const parent = row({ id: "main-1", device_id: "dev-peer" })
    const child = row({ id: "agent-a", parent_session_id: "main-1" }) // dev-self
    const { rows, nestedKeys } = nestSubagents([parent, child])
    expect(rows.map((r) => r.id)).toEqual(["main-1", "agent-a"])
    expect(nestedKeys.size).toBe(0)
  })

  it("returns rows untouched when no row carries a link", () => {
    const a = row({ id: "a" })
    const b = row({ id: "b" })
    const { rows, nestedKeys } = nestSubagents([a, b])
    expect(rows).toEqual([a, b])
    expect(nestedKeys.size).toBe(0)
  })
})
