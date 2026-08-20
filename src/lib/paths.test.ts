// Tests for the shared path-display derivations (moved from
// features/sessions/derive.test.ts when projectBasename became a cross-feature
// lib — the project filter dropdown shares it with the sessions surfaces).

import { describe, expect, it } from "vitest"

import { projectBasename } from "./paths"

describe("projectBasename", () => {
  it("takes the final path component on both separators", () => {
    expect(projectBasename("D:\\Project\\O_CC_One")).toBe("O_CC_One")
    expect(projectBasename("/home/user/vault-one")).toBe("vault-one")
    expect(projectBasename("solo")).toBe("solo")
    expect(projectBasename("D:\\proj\\")).toBe("proj")
  })

  it("returns an all-separator or empty path as-is (caller renders the placeholder)", () => {
    expect(projectBasename("")).toBe("")
    expect(projectBasename("\\/")).toBe("")
  })
})
