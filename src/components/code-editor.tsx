// Reusable CodeMirror 6 code editor. The shared core — mount-once view, chrome
// theme, two-way value sync (no echo on programmatic pushes), theme/placeholder/
// editable compartments — lives here once. JSON mode adds the rich layer
// (`@codemirror/lang-json` syntax + lint, paste→format expand, format button,
// object validation strip); TOML mode is highlight-only (`@codemirror/legacy-
// modes` StreamLanguage) — no client-side TOML parser, so client-side format /
// validation are JSON-only and TOML validity is checked by the backend. TOML
// 的整理走后端 taplo（ADR-0011）：粘贴 / 外部值进入 / 「整理」按钮都调
// format_toml_cmd（保注释、容错，失败保持原文）。
//
// `JsonEditor` is a thin `language="json"` wrapper over this (unchanged API for
// existing consumers); the snippet card uses `language="toml"` for codex/grok.

import { json, jsonParseLinter } from "@codemirror/lang-json"
import { StreamLanguage } from "@codemirror/language"
import { toml } from "@codemirror/legacy-modes/mode/toml"
import { linter } from "@codemirror/lint"
import { Compartment, EditorState } from "@codemirror/state"
import { oneDark } from "@codemirror/theme-one-dark"
import { placeholder } from "@codemirror/view"
import { basicSetup, EditorView } from "codemirror"
import { AlertCircle, Wand2 } from "lucide-react"
import { useTheme } from "next-themes"
import { useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { useFormatTomlMutation } from "@/app/store/api"
import { Button } from "@/components/ui/button"
import { parseJsonObject, tidyJson } from "@/lib/json"
import { cn } from "@/lib/utils"

export type CodeLanguage = "json" | "toml"

export interface CodeEditorProps {
  /** The current text (controlled). */
  value: string
  /** Emitted with the new text on user edits and after formatting. */
  onChange: (value: string) => void
  /** Editor language: json enables syntax + lint + format; toml is highlight-only. */
  language: CodeLanguage
  disabled?: boolean
  placeholder?: string
  /** Height / width classes for the wrapping block (e.g. `h-72`). */
  className?: string
}

// Chrome shared by both modes — border/radius from the shadcn CSS variables so
// the editor sits inside a Sheet like any other input. Background is left to
// the mode theme (transparent in light, oneDark's panel in dark).
const cmChrome = EditorView.theme({
  "&": {
    height: "100%",
    // 最小高度小于外层容器常见的固定高度（如 h-40 = 160px），让 .cm-editor
    // 的 height:100% 能随 flex-1 的 container 收缩，给底部的格式化按钮留出
    // 位置——否则编辑区被 minHeight 撑满，按钮溢出盖到下方内容上。
    minHeight: "6rem",
    fontSize: "13px",
    border: "1px solid hsl(var(--border))",
    borderRadius: "calc(var(--radius) - 2px)",
  },
  "&.cm-focused": {
    outline: "none",
    borderColor: "hsl(var(--ring))",
  },
  ".cm-scroller": {
    overflow: "auto",
    fontFamily:
      "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace",
    lineHeight: "1.5",
  },
  ".cm-gutters": {
    backgroundColor: "transparent",
    borderRight: "1px solid hsl(var(--border))",
    color: "hsl(var(--muted-foreground) / 0.6)",
  },
  ".cm-content": {
    caretColor: "hsl(var(--foreground))",
  },
  ".cm-cursor": {
    borderLeftColor: "hsl(var(--foreground))",
  },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    {
      backgroundColor: "hsl(var(--primary) / 0.16)",
    },
  ".cm-activeLine": {
    backgroundColor: "hsl(var(--primary) / 0.06)",
  },
  ".cm-activeLineGutter": {
    backgroundColor: "hsl(var(--primary) / 0.06)",
  },
})

/** JSON-only lint source: the non-object check (syntax errors are already
 *  reported by `jsonParseLinter` with precise positions, so it runs its own
 *  JSON.parse). */
function objectLinter(mustBeObject: string) {
  return linter((view) => {
    const doc = view.state.doc.toString()
    if (!doc.trim()) return []
    let parsed: unknown
    try {
      parsed = JSON.parse(doc)
    } catch {
      return []
    }
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return [
        { from: 0, to: doc.length, severity: "error", message: mustBeObject },
      ]
    }
    return []
  })
}

export function CodeEditor({
  value,
  onChange,
  language,
  disabled = false,
  placeholder: placeholderText,
  className,
}: CodeEditorProps) {
  const { t } = useTranslation()
  const { resolvedTheme } = useTheme()
  // Default to dark before next-themes resolves — the app default theme is
  // dark, so a dark-first render avoids flashing the light editor.
  const isDark = (resolvedTheme ?? "dark") === "dark"
  // JSON gets the rich layer; TOML is highlight-only (no client parser → no
  // format / paste-expand / validation strip, see file header). TOML 的整理走
  // 后端 taplo（ADR-0011）：ref 保存 mutation trigger（每次渲染可能换引用），
  // 供 mount-once 的 updateListener / effects 闭包安全调用。
  const isJson = language === "json"
  const [formatToml] = useFormatTomlMutation()
  const formatTomlRef = useRef(formatToml)
  formatTomlRef.current = formatToml

  const containerRef = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  // True only while a programmatic dispatch is pushing an external value in —
  // the update listener skips emitting while this is set, breaking the echo.
  const pushingRef = useRef(false)
  // Validation message shown under the editor (JSON only): the linter pinpoints
  // the location, the strip says what failed. Mirrors the editor's own doc —
  // user edits land in `value` via onChange → parent, so checking on [value]
  // covers both paths.
  const [validationError, setValidationError] = useState<string | null>(null)

  const themeCompartment = useMemo(() => new Compartment(), [])
  const langCompartment = useMemo(() => new Compartment(), [])
  const editableCompartment = useMemo(() => new Compartment(), [])
  const placeholderCompartment = useMemo(() => new Compartment(), [])

  // Language + linters. Rebuilt when the language or the translated message
  // changes so a switch never leaves a stale linter string. TOML has no client
  // linter (validity checked server-side).
  const langExtensions = useMemo(() => {
    if (!isJson) return [StreamLanguage.define(toml)]
    return [
      json(),
      linter(jsonParseLinter()),
      objectLinter(t("jsonEditor.mustBeObject")),
    ]
  }, [isJson, t])

  // Create the editor once, with an empty doc — the value-sync effect below
  // pushes the real initial value right after (same commit, no visible flash).
  // Everything reactive reconfigures through the compartments in the effects
  // below, so the editor mounts exactly once. The onChange callback lives in a
  // ref so this mount-once closure always sees the latest prop.
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange
  // biome-ignore lint/correctness/useExhaustiveDependencies: mount-once CodeMirror view; all reactive inputs reconfigure through compartments below
  useEffect(() => {
    if (!containerRef.current) return
    const view = new EditorView({
      state: EditorState.create({
        doc: "",
        extensions: [
          basicSetup,
          cmChrome,
          themeCompartment.of(isDark ? [oneDark] : []),
          langCompartment.of(langExtensions),
          editableCompartment.of([
            EditorState.readOnly.of(disabled),
            EditorView.editable.of(!disabled),
          ]),
          placeholderCompartment.of(
            placeholderText ? [placeholder(placeholderText)] : [],
          ),
          EditorView.updateListener.of((update) => {
            if (!update.docChanged || pushingRef.current) return
            // 粘贴（含「全部替换」式插入）即自动展开：与外部值进入同一规则。
            // JSON 本地整理（formatJson 容错，语法错误 / 尾逗号等无效 JSON 也
            // 展开成可读结构，字符串字面量不受影响）；TOML 调后端 taplo
            // （ADR-0011「输入后自动触发」，保注释、容错）。先 dispatch 再回传
            // 格式化结果，父级状态一次到位，不会闪过原文。TOML 异步整理回调时
            // 若用户已继续编辑（doc 不再等于粘贴快照）不强推，只回传当前内容。
            // 错误位置由 jsonParseLinter 的红线标记，这里只负责排版。
            if (
              update.transactions.some((tr) => tr.isUserEvent("input.paste"))
            ) {
              const text = update.view.state.doc.toString()
              if (isJson) {
                const formatted = tidyJson(text)
                if (formatted !== text) {
                  pushingRef.current = true
                  try {
                    update.view.dispatch({
                      changes: { from: 0, to: text.length, insert: formatted },
                    })
                  } finally {
                    pushingRef.current = false
                  }
                }
                onChangeRef.current(formatted)
                return
              }
              void formatTomlRef.current(text).then((res) => {
                const formatted = res.error ? text : res.data
                if (formatted === text) {
                  onChangeRef.current(text)
                  return
                }
                const view = viewRef.current
                if (!view || view.state.doc.toString() !== text) {
                  // 用户已继续编辑：不覆盖，只保证父级拿到粘贴原文。
                  onChangeRef.current(text)
                  return
                }
                pushingRef.current = true
                try {
                  view.dispatch({
                    changes: { from: 0, to: text.length, insert: formatted },
                  })
                } finally {
                  pushingRef.current = false
                }
                onChangeRef.current(formatted)
              })
              return
            }
            onChangeRef.current(update.state.doc.toString())
          }),
        ],
      }),
      parent: containerRef.current,
    })
    viewRef.current = view
    return () => {
      view.destroy()
      viewRef.current = null
    }
  }, [])

  // External value → push into the editor without echoing back. No-op when the
  // doc already holds the value (covers the editor's own edits round-tripping
  // through the parent). A compact JSON external value is expanded to multi-line
  // on the way in (JSON only) — opening the form shows a readable structure
  // instead of a single compressed line. formatJson is lenient (jsonc-parser):
  // even broken JSON spreads into an outline, never throws. TOML external values
  // go through the backend taplo formatter (ADR-0011 auto-trigger) — async, so
  // the callback skips if the user already edited (doc no longer matches the
  // snapshot) and never overwrites their typing. The editor's own edits never
  // reach this path (cur === value).
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    const cur = view.state.doc.toString()
    if (cur === value) return
    if (isJson) {
      let insert = value
      if (!pushingRef.current) {
        const formatted = tidyJson(value)
        if (formatted !== value) insert = formatted
      }
      pushingRef.current = true
      try {
        view.dispatch({ changes: { from: 0, to: cur.length, insert } })
      } finally {
        pushingRef.current = false
      }
      if (insert !== value) onChangeRef.current(insert)
      return
    }
    // TOML：外部值进入（打开表单 / 保存回读 / T6 提取后）即调后端整理。容错：
    // 失败保持原文；回调时 doc 已不等于进入时的快照 → 用户编辑过，不强推。
    void formatTomlRef.current(value).then((res) => {
      const formatted = res.error ? value : res.data
      const v = viewRef.current
      if (!v || v.state.doc.toString() !== cur) return
      if (formatted === value) return
      pushingRef.current = true
      try {
        v.dispatch({ changes: { from: 0, to: cur.length, insert: formatted } })
      } finally {
        pushingRef.current = false
      }
      onChangeRef.current(formatted)
    })
  }, [value, isJson])

  // Refresh the validation strip whenever the controlled text changes (JSON
  // only — TOML validity is checked by the backend). User edits round-trip
  // through `value`; programmatic pushes set it directly.
  useEffect(() => {
    if (!isJson) {
      setValidationError(null)
      return
    }
    const result = parseJsonObject(value)
    setValidationError(result.ok ? null : result.error)
  }, [value, isJson])

  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: themeCompartment.reconfigure(isDark ? [oneDark] : []),
    })
  }, [isDark, themeCompartment])

  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({ effects: langCompartment.reconfigure(langExtensions) })
  }, [langExtensions, langCompartment])

  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: editableCompartment.reconfigure([
        EditorState.readOnly.of(disabled),
        EditorView.editable.of(!disabled),
      ]),
    })
  }, [disabled, editableCompartment])

  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.dispatch({
      effects: placeholderCompartment.reconfigure(
        placeholderText ? [placeholder(placeholderText)] : [],
      ),
    })
  }, [placeholderText, placeholderCompartment])

  /** 统一「整理」：按语言分派——JSON 本地 tidyJson（格式化+排序）；TOML 调
   *  后端 format_toml_cmd（taplo 保注释格式化，见 ADR-0011）。TOML 是异步：
   *  dispatch 结果时用 pushingRef 防回声。 */
  function handleFormat() {
    const view = viewRef.current
    if (!view) return
    const text = view.state.doc.toString()
    if (!text.trim()) return
    const apply = (formatted: string) => {
      if (formatted === text) return
      pushingRef.current = true
      try {
        view.dispatch({
          changes: { from: 0, to: text.length, insert: formatted },
        })
      } finally {
        pushingRef.current = false
      }
      onChange(formatted)
    }
    if (isJson) {
      apply(tidyJson(text))
    } else {
      void formatTomlRef.current(text).then((res) => {
        // mutation 容错（run 归一）：失败保持原文（整理是容错的，不弹错）。
        if (!res.error) apply(res.data)
      })
    }
  }

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div
        ref={containerRef}
        className="min-h-24 min-w-0 flex-1 overflow-hidden"
      />
      {validationError ? (
        <p className="bg-destructive/10 text-destructive flex items-start gap-1.5 rounded-md px-2.5 py-1.5 text-xs">
          <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
          <span>{validationError}</span>
        </p>
      ) : null}
      <div className="flex items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="xs"
          disabled={disabled}
          onClick={handleFormat}
          title={t("jsonEditor.formatHint")}
        >
          <Wand2 />
          {t("jsonEditor.format")}
        </Button>
      </div>
    </div>
  )
}
