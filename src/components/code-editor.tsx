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
// 所有「整理后写回」的路径（粘贴 / 外部值进入 / 「整理」按钮）共用同一条管
// 道：formatDoc 按语言选整理策略（JSON 本地 tidyJson，TOML 后端 taplo），
// planDocApply + applyFormatted 统一执行写回——防回声与「用户编辑优先」守卫
// 对两种语言由构造同一，语言差异只存在于 formatDoc 内部。
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
import { Wand2 } from "lucide-react"
import { useTheme } from "next-themes"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"

import { useFormatTomlMutation } from "@/app/store/api"
import { InlineBanner } from "@/components/inline-banner"
import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
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

/** applyFormatted 的决策核（纯函数，所有写回路径共用）。snapshot 是发起整
 *  理时读到的 doc 快照，对照编辑器当前 doc 与整理结果判三态：
 *  - "stale"：doc 已不等于快照 → 用户在异步整理间隙输入过。用户的每次击键
 *    都已实时回传父级，这里任何补写都只会覆盖或倒退，必须放弃（不写也不
 *    回传）。
 *  - "write"：doc 仍是快照且整理改变了内容 → 整篇替换并回传。
 *  - "settle"：doc 已是目标形（幂等：后端整理无变化 / 失败回原文 / JSON 已
 *    展开）→ 不发空事务，但仍回传一次——粘贴路径的粘贴原文从未经普通编辑
 *    回传父级，必须补；其余路径父级已持同值，setState Object.is 判等直接
 *    收敛，无副作用。 */
export type DocApplyPlan = "stale" | "write" | "settle"

export function planDocApply(
  docNow: string,
  snapshot: string,
  formatted: string,
): DocApplyPlan {
  if (docNow !== snapshot) return "stale"
  return formatted === snapshot ? "settle" : "write"
}

/** formatDoc 的 TOML 整理后端（注入面）：生产传 formatToml 的 mutation
 *  trigger，测试传 fake。data / error 二选一（RTK mutation 结果形状）。 */
export type FormatTomlBackend = (text: string) => Promise<{
  data?: string
  error?: unknown
}>

/** 按语言把文本整理为规范形（策略入口，所有写回路径共用）：JSON 走本地
 *  tidyJson（容错排版 + 键字母序排序，见 lib/json，同步）；TOML 走后端
 *  format_toml_cmd（taplo 保注释，ADR-0011，异步）。容错契约：不抛错、总
 *  返回一份完整可写回的文本——TOML 失败（error 或无 data）返回原文，调用
 *  管道因此不需要失败分支。 */
export async function formatDoc(
  language: CodeLanguage,
  text: string,
  formatToml: FormatTomlBackend,
): Promise<string> {
  if (language === "json") return tidyJson(text)
  const res = await formatToml(text)
  return res.data ?? text
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
  // format / paste-expand / validation strip, see file header). 语言差异全部
  // 收敛进 formatDoc（整理策略）；写回策略由 applyFormatted 对两种语言统一。
  const isJson = language === "json"
  // TOML 整理后端（formatDoc 的注入值）：ref 保存 mutation trigger（每次渲染
  // 可能换引用），供 mount-once 的 updateListener / effects 闭包安全调用。
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

  // The onChange callback lives in a ref so mount-once closures (the update
  // listener below and applyFormatted here) always see the latest prop.
  const onChangeRef = useRef(onChange)
  onChangeRef.current = onChange

  // 整理写回管道的执行半边（决策半边是模块级的 planDocApply）。闭包只经 ref
  // 读活值（viewRef / pushingRef / onChangeRef），mount-once 的 update
  // listener 持有的首帧引用因此始终正确。
  const applyFormatted = useCallback((snapshot: string, formatted: string) => {
    const view = viewRef.current
    if (!view) return
    const plan = planDocApply(view.state.doc.toString(), snapshot, formatted)
    if (plan === "stale") return
    if (plan === "write") {
      pushingRef.current = true
      try {
        view.dispatch({
          changes: { from: 0, to: snapshot.length, insert: formatted },
        })
      } finally {
        pushingRef.current = false
      }
    }
    // settle（幂等）也回传：见 planDocApply 的三态说明。
    onChangeRef.current(formatted)
  }, [])

  // Create the editor once, with an empty doc — the value-sync effect below
  // pushes the real initial value right after (same commit, no visible flash).
  // Everything reactive reconfigures through the compartments in the effects
  // below, so the editor mounts exactly once.
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
            // 粘贴（含「全部替换」式插入）即自动展开：与外部值进入、「整理」
            // 按钮同一条 formatDoc → applyFormatted 管道。JSON 本地整理（容
            // 错，语法错误 / 尾逗号等无效 JSON 也展开成可读结构，字符串字面
            // 量不受影响）；TOML 调后端 taplo（ADR-0011「输入后自动触发」，
            // 保注释、容错）。异步回调里用户若已继续输入，applyFormatted 的
            // stale 守卫放弃写回——击键已实时回传父级，内容不丢；整理未改变
            // 内容则 settle 幂等补回传，父级状态一次到位，不闪原文。错误位
            // 置由 jsonParseLinter 的红线标记，这里只负责排版。
            if (
              update.transactions.some((tr) => tr.isUserEvent("input.paste"))
            ) {
              const text = update.view.state.doc.toString()
              void formatDoc(language, text, formatTomlRef.current).then(
                (formatted) => applyFormatted(text, formatted),
              )
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
  // through the parent). Both languages enter the same formatDoc →
  // applyFormatted pipeline as paste and the format button: the incoming value
  // is normalized on the way in (JSON tidy-expanded locally, TOML through the
  // backend taplo — failure keeps the original text), and a user who typed
  // during the async round-trip wins (stale guard): their keystrokes already
  // reported the newer content to the parent, so the push must not clobber it.
  // The editor's own edits never reach this path (cur === value).
  useEffect(() => {
    const view = viewRef.current
    if (!view) return
    const cur = view.state.doc.toString()
    if (cur === value) return
    void formatDoc(language, value, formatTomlRef.current).then((formatted) =>
      applyFormatted(cur, formatted),
    )
  }, [value, language, applyFormatted])

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

  /** 「整理」按钮：读当前 doc → formatDoc 按语言整理 → applyFormatted 写回，
   *  与粘贴 / 外部值进入共用同一管道。整理失败（formatDoc 返回原文）落为
   *  settle 幂等：无事务、同值回传由 setState 判等收敛；异步整理期间用户
   *  已输入则 stale 守卫放弃写回，不覆盖。 */
  async function handleFormat() {
    const view = viewRef.current
    if (!view) return
    const text = view.state.doc.toString()
    if (!text.trim()) return
    const formatted = await formatDoc(language, text, formatTomlRef.current)
    applyFormatted(text, formatted)
  }

  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div
        ref={containerRef}
        className="min-h-24 min-w-0 flex-1 overflow-hidden"
      />
      {validationError ? (
        <InlineBanner tone="error">{validationError}</InlineBanner>
      ) : null}
      <div className="flex items-center gap-2">
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type="button"
                variant="outline"
                size="xs"
                disabled={disabled}
                onClick={() => void handleFormat()}
              >
                <Wand2 />
                {t("jsonEditor.format")}
              </Button>
            }
          />
          <TooltipContent>{t("jsonEditor.formatHint")}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  )
}
