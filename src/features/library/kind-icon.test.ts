import {
  File as FileIcon,
  FileJson,
  FileText,
  Folder,
  Image as ImageIcon,
} from "lucide-react"
import { describe, expect, it } from "vitest"

import { kindIcon } from "./kind-icon"

describe("kindIcon", () => {
  it("returns the folder icon for directories", () => {
    expect(kindIcon("notes", true)).toBe(Folder)
    expect(kindIcon("assets.json", true)).toBe(Folder)
  })

  it("maps json / text-ish / image extensions case-insensitively", () => {
    expect(kindIcon("config.JSON", false)).toBe(FileJson)
    expect(kindIcon("README.md", false)).toBe(FileText)
    expect(kindIcon("doc.markdown", false)).toBe(FileText)
    expect(kindIcon("session.LOG", false)).toBe(FileText)
    expect(kindIcon("log.txt", false)).toBe(FileText)
    expect(kindIcon("photo.PNG", false)).toBe(ImageIcon)
    expect(kindIcon("drawing.svg", false)).toBe(ImageIcon)
  })

  it("falls back to the generic file icon for unknown extensions and extensionless files", () => {
    expect(kindIcon("script.py", false)).toBe(FileIcon)
    expect(kindIcon("no-extension", false)).toBe(FileIcon)
    expect(kindIcon(".gitkeep", false)).toBe(FileIcon)
  })
})
