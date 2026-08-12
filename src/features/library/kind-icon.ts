// Extension → icon mapping, shared by the library table and the upload
// dialog. Single source of truth — the two call sites used to carry
// duplicate copies that had already drifted apart (an extensionless name was
// a file in the view but a folder in the dialog).

import {
  File as FileIcon,
  FileJson,
  FileText,
  Folder,
  Image as ImageIcon,
  type LucideIcon,
} from "lucide-react"

/** Icon for a library / upload row. `isDir` is authoritative when the caller
 *  knows the kind (the table does, from entry.kind); the upload dialog only
 *  has a path, so it passes its best guess (no extension ⇒ folder). */
export function kindIcon(name: string, isDir: boolean): LucideIcon {
  if (isDir) return Folder
  const ext = name.split(".").pop()?.toLowerCase()
  if (!ext) return FileIcon
  if (ext === "json") return FileJson
  if (["md", "markdown", "txt", "log"].includes(ext)) return FileText
  if (["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"].includes(ext))
    return ImageIcon
  return FileIcon
}
