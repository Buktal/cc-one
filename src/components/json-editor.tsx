// JSON editor — a thin `language="json"` wrapper over `CodeEditor` (the shared
// CodeMirror core: chrome, two-way sync, theme, language layer). Keeps the
// historical `JsonEditor` name and props so existing consumers (provider form
// sheet, claude/gemini snippet card) are untouched. TOML editing goes straight
// to `CodeEditor` with `language="toml"`.

import { CodeEditor, type CodeEditorProps } from "@/components/code-editor"

export type JsonEditorProps = Omit<CodeEditorProps, "language">

export function JsonEditor(props: JsonEditorProps) {
  return <CodeEditor language="json" {...props} />
}
