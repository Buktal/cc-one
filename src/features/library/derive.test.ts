import { describe, expect, it } from "vitest"

import { filterEntriesByName, shouldThemeRender } from "./derive"

describe("shouldThemeRender", () => {
  it("renders text extensions theme-side, case-insensitively", () => {
    for (const name of [
      "notes.md",
      "README.MD",
      "config.json",
      "archive.JSON",
      "doc.markdown",
      "log.txt",
      "session.log",
    ]) {
      expect(shouldThemeRender(name), name).toBe(true)
    }
  })

  it("keeps iframe rendering for html / pdf / svg / unknown / extensionless", () => {
    for (const name of [
      "page.html",
      "manual.pdf",
      "image.svg",
      "script.py",
      "no-extension",
      "archive.tar.gz",
      ".gitkeep",
    ]) {
      expect(shouldThemeRender(name), name).toBe(false)
    }
  })
})

describe("filterEntriesByName", () => {
  const entries = [
    { name: "notes.md" },
    { name: "Notes" },
    { name: "config.json" },
    { name: "README.md" },
  ]

  it("matches names case-insensitively", () => {
    expect(filterEntriesByName(entries, "notes")).toEqual([
      { name: "notes.md" },
      { name: "Notes" },
    ])
  })

  it("returns the list untouched for a blank query", () => {
    expect(filterEntriesByName(entries, "  ")).toBe(entries)
  })

  it("returns nothing when no entry matches", () => {
    expect(filterEntriesByName(entries, "zzz")).toEqual([])
  })

  it("trims the query", () => {
    expect(filterEntriesByName(entries, "  README  ")).toEqual([
      { name: "README.md" },
    ])
  })
})
