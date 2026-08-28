// Tests for the request-log browser controller.
//
// The controller wires usePagedBrowser (pure state machine, covered in
// use-paged-browser.test.ts) + the count/logs queries + expanded-row state.
// vitest runs in a pure node environment (see vitest.config.ts — no DOM), so
// renderHook is out of scope; what we guard here is that the module imports
// cleanly in node (it pulls the tauri-specta API + RTK Query hooks) — the
// family's standard smoke guard (mirrors use-sessions-browser.test.ts).

import { describe, expect, it } from "vitest"

describe("useRequestLogBrowser imports in a non-Tauri (node) environment", () => {
  it("imports without throwing and exports a function", async () => {
    const mod = await import("./use-request-log-browser")
    expect(typeof mod.useRequestLogBrowser).toBe("function")
  })
})
