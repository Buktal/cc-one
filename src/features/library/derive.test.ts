import { describe, expect, it } from "vitest"

import {
  extOf,
  filterEntriesByName,
  isImageName,
  maybePrettyJson,
  shouldThemeRender,
} from "./derive"

describe("extOf", () => {
  it("returns the lowercase extension without the dot", () => {
    expect(extOf("notes.md")).toBe("md")
    expect(extOf("README.MD")).toBe("md")
    expect(extOf("archive.tar.gz")).toBe("gz")
    expect(extOf("dotted.name.json")).toBe("json")
  })

  it("returns empty for an extensionless name", () => {
    expect(extOf("LICENSE")).toBe("")
    expect(extOf("Makefile")).toBe("")
  })

  it("treats a leading dot as a separator (.gitkeep → gitkeep)", () => {
    // lastIndexOf 语义自 shouldThemeRender 时代沿用：名字里有点即有扩展名，
    // 点缀文件名与普通扩展名同判——kindIcon(".gitkeep") 由此落回 file 图标。
    expect(extOf(".gitkeep")).toBe("gitkeep")
  })
})

describe("isImageName", () => {
  it("matches the image table case-insensitively", () => {
    for (const name of [
      "photo.PNG",
      "shot.jpg",
      "raw.jpeg",
      "anim.gif",
      "pic.webp",
      "drawing.svg",
      "scan.bmp",
    ]) {
      expect(isImageName(name), name).toBe(true)
    }
  })

  it("rejects non-image and extensionless names", () => {
    for (const name of ["notes.md", "page.html", "script.py", "no-extension"]) {
      expect(isImageName(name), name).toBe(false)
    }
  })
})

describe("maybePrettyJson", () => {
  it("pretty-prints valid JSON with 2-space indent", () => {
    expect(maybePrettyJson('{"a":1,"b":[2,3]}')).toBe(
      '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ]\n}',
    )
  })

  it("falls back to the raw text on parse failure (JSONL / non-JSON body)", () => {
    const raw = '{"a":1}\n{"a":2}\n'
    expect(maybePrettyJson(raw)).toBe(raw)
    expect(maybePrettyJson("not json at all")).toBe("not json at all")
  })
})

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
