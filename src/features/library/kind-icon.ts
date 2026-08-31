// Extension → icon mapping, shared by the library table and the upload
// dialog. Single source of truth for icons; the filename parsing itself is
// delegated to ./derive (extOf / shouldThemeRender / isImageName) — the call
// sites used to hand-roll the parse and its case handling, and the copies had
// drifted (an extensionless name displayed as its own uppercased name in the
// kind column; the dialog's "no extension ⇒ folder" guess silently missed
// non-lowercase names).
//
// The text branch shares the preview's theme-text predicate on purpose: what
// renders as theme-styled text gets the text icon, so icon and preview cannot
// disagree about a name. JSON is branched off first for its dedicated icon.

import {
  File as FileIcon,
  FileJson,
  FileText,
  Folder,
  Image as ImageIcon,
  type LucideIcon,
} from "lucide-react"
import { extOf, isImageName, shouldThemeRender } from "./derive"

/** Icon for a library / upload row. `isDir` is authoritative when the caller
 *  knows the kind (the table does, from entry.kind); the upload dialog only
 *  has a path, so it passes its best guess (no extension ⇒ folder). */
export function kindIcon(name: string, isDir: boolean): LucideIcon {
  if (isDir) return Folder
  if (extOf(name) === "json") return FileJson
  if (shouldThemeRender(name)) return FileText
  if (isImageName(name)) return ImageIcon
  return FileIcon
}
