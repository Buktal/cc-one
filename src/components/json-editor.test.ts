// Tests for the code editor components.
//
// The editor mounts a CodeMirror 6 view, which needs a DOM, so vitest (pure
// node, no DOM) guards only that the module imports cleanly — the same failure
// mode the browser hooks guard against (a regression that hoists a DOM handle
// to module top level would make the component un-importable and zero-tested).

import { describe, expect, it } from "vitest"

describe("editors import in a non-DOM (node) environment", () => {
  it("JsonEditor imports without throwing and exports a component", async () => {
    const mod = await import("@/components/json-editor")
    expect(typeof mod.JsonEditor).toBe("function")
  })

  it("CodeEditor imports without throwing and exports a component", async () => {
    const mod = await import("@/components/code-editor")
    expect(typeof mod.CodeEditor).toBe("function")
  })
})
