// Tests for the library browser.
//
// The navigation rules (splitEntryPath / upFromSubpath / buildBreadcrumb) are
// pure functions in ./derive, covered directly below — they encode the
// drill / go-up / breadcrumb invariants the view used to inline (architecture
// .md: "关键不变量用代码表达"). useLibraryBrowser itself just wires those
// derivations to React state + RTK Query; vitest runs in a pure node
// environment (see vitest.config.ts — no DOM), so renderHook is out of scope.
// What we guard at the bottom is that the hook module imports cleanly in node
// (it pulls @tauri-apps/api/webview + plugin-dialog + the tauri-specta API) —
// a regression that moves the getCurrentWebview() fetch to module top level
// would otherwise make the hook un-importable and zero-tested, the same
// failure mode that once hid the shell-hooks bug.

import { describe, expect, it } from "vitest"

import type { DeviceOption } from "./derive"
import { buildBreadcrumb, splitEntryPath, upFromSubpath } from "./derive"

describe("splitEntryPath", () => {
  it("splits a top-level entry into device id + empty rest", () => {
    expect(splitEntryPath("dev1")).toEqual({ deviceId: "dev1", rest: "" })
  })

  it("splits a nested entry into device id + relative subpath", () => {
    expect(splitEntryPath("dev1/projects/a.json")).toEqual({
      deviceId: "dev1",
      rest: "projects/a.json",
    })
  })
})

describe("upFromSubpath", () => {
  // subpath never carries the device id (drill + the scan query treat
  // deviceScope and subpath as separate values), so going up only ever drops
  // the last subpath segment — deviceScope is untouched.

  it("clears to the device root for a single segment", () => {
    expect(upFromSubpath("projects")).toBe("")
  })

  it("drops only the last segment for two segments", () => {
    expect(upFromSubpath("projects/foo")).toBe("projects")
  })

  it("drops only the last segment for three or more segments", () => {
    expect(upFromSubpath("a/b/c")).toBe("a/b")
  })

  it("clears for empty input (defensive — goUp is hidden at root)", () => {
    expect(upFromSubpath("")).toBe("")
  })
})

describe("buildBreadcrumb", () => {
  const deviceOptions: DeviceOption[] = [
    { id: "dev1", label: "Device One" },
    { id: "dev2", label: "Device Two" },
  ]

  it("returns no crumbs at the root (empty subpath)", () => {
    expect(buildBreadcrumb("dev1", "", deviceOptions)).toEqual([])
  })

  it("labels the device crumb from deviceScope, with one crumb per subpath segment", () => {
    expect(buildBreadcrumb("dev1", "projects", deviceOptions)).toEqual([
      { key: "dev1", label: "Device One", deviceScope: "dev1", subpath: "" },
      {
        key: "dev1/projects",
        label: "projects",
        deviceScope: "dev1",
        subpath: "projects",
      },
    ])
  })

  it("keeps deviceScope constant across every crumb in a deep trail", () => {
    expect(buildBreadcrumb("dev1", "projects/foo", deviceOptions)).toEqual([
      { key: "dev1", label: "Device One", deviceScope: "dev1", subpath: "" },
      {
        key: "dev1/projects",
        label: "projects",
        deviceScope: "dev1",
        subpath: "projects",
      },
      {
        key: "dev1/projects/foo",
        label: "foo",
        deviceScope: "dev1",
        subpath: "projects/foo",
      },
    ])
  })

  it("falls back to the raw id when the device is not in deviceOptions", () => {
    expect(buildBreadcrumb("ghost", "x", deviceOptions)).toEqual([
      { key: "ghost", label: "ghost", deviceScope: "ghost", subpath: "" },
      { key: "ghost/x", label: "x", deviceScope: "ghost", subpath: "x" },
    ])
  })
})

describe("useLibraryBrowser imports in a non-Tauri (node) environment", () => {
  it("imports without throwing and exports a function", async () => {
    const mod = await import("./use-library-browser")
    expect(typeof mod.useLibraryBrowser).toBe("function")
  })

  it("does not export the ALL sentinel — the caller domain stays empty-string", async () => {
    // 哨兵收敛进共享 FilterSelect 后（见 @/components/filter-select 与
    // @/lib/filter-options），hook 只接触「空串 = 全部设备」域；若有人把
    // 哨兵重新导出到调用方，即破坏收敛边界。
    const mod = await import("./use-library-browser")
    expect("ALL" in mod).toBe(false)
  })
})
