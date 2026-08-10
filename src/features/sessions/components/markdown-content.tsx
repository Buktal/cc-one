// Markdown rendering for transcript bubbles (assistant / user / system rows)
// and tool rows' JSON payloads. react-markdown parses to React elements —
// raw HTML/XML in a message is escaped to text, never executed — with the
// GFM plugin (tables / strikethrough / task lists), remark-breaks (a pasted
// log's single newlines must survive, CommonMark would merge them), and
// rehype-highlight for code blocks.
//
// Two rendering surfaces: inline `code` gets a tinted chip, block code gets
// the shared CodeBlock — a fixed dark pane (mode-independent, like chat apps)
// with a language tag and internal scrolling for long output. Links open via
// the system opener instead of the webview. Typography lives in .md-body /
// .md-codeblock (index.css) so the theme variables drive the colors.

import { openUrl } from "@tauri-apps/plugin-opener"
import { memo, type ReactNode } from "react"
import ReactMarkdown from "react-markdown"
import rehypeHighlight from "rehype-highlight"
import remarkBreaks from "remark-breaks"
import remarkGfm from "remark-gfm"
import { tryFormatJson } from "../derive"

/** A rendered message body. Memoized — the transcript is virtualized, so
 *  rows unmount and remount while scrolling; identical text must not re-parse. */
export const MarkdownContent = memo(function MarkdownContent({
  text,
}: {
  text: string
}) {
  return (
    <div className="md-body">
      <ReactMarkdown
        remarkPlugins={[remarkGfm, remarkBreaks]}
        // detect: unlabeled fenced blocks (common in pasted user messages) get
        // auto-detected against highlight.js's common subset instead of
        // staying plain.
        rehypePlugins={[[rehypeHighlight, { detect: true }]]}
        components={{ code: InlineOrBlockCode, a: MdLink }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
})

/** Inline `code` (no language, no newline) → tinted chip; everything else →
 *  the shared CodeBlock. react-markdown marks fenced blocks with a
 *  `language-xxx` class on the code element, so the absence of one (plus a
 *  single-line body) is the inline heuristic. */
function InlineOrBlockCode({
  className,
  children,
}: {
  className?: string
  children?: ReactNode
}) {
  const match = /language-([\w-]+)/.exec(className ?? "")
  const isInline = !match && !String(children).includes("\n")
  if (isInline) {
    return (
      <code className="bg-foreground/10 rounded px-1.5 py-0.5 font-mono text-[0.85em]">
        {children}
      </code>
    )
  }
  return <CodeBlock language={match?.[1]}>{children}</CodeBlock>
}

/** Dark code pane with a language tag and internal scroll. Shared by markdown
 *  code blocks and tool rows' formatted JSON (language="json"). */
export function CodeBlock({
  language,
  children,
}: {
  language?: string
  children?: ReactNode
}) {
  return (
    <div className="md-codeblock">
      {/* Header + lang label are colored via .md-codeblock-header / -lang so
        they follow the mode (see index.css). */}
      <div className="md-codeblock-header">
        <span className="md-codeblock-lang font-mono">
          {language ?? "code"}
        </span>
      </div>
      <pre className="max-h-96 overflow-auto p-3">
        <code className="font-mono text-[12.5px] leading-relaxed">
          {children}
        </code>
      </pre>
    </div>
  )
}

/** A tool row's expanded body: JSON (tool_use inputs are JSON strings) gets
 *  pretty-printed into a highlighted json pane; anything else renders as plain
 *  monospace text (a raw markdown pass would misparse `{`-heavy content). */
export function ToolContent({ text }: { text: string }) {
  const formatted = tryFormatJson(text)
  if (formatted !== null) {
    return <CodeBlock language="json">{formatted}</CodeBlock>
  }
  return (
    <div className="text-muted-foreground mt-1.5 rounded bg-background/60 p-2 font-mono break-words whitespace-pre-wrap">
      {text}
    </div>
  )
}

/** Links open in the system browser (Tauri webview has no native navigation);
 *  fall back to a new window when the opener is unavailable (tests / plain
 *  browser context). */
function MdLink({ href, children }: { href?: string; children?: ReactNode }) {
  if (!href) return <span>{children}</span>
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault()
        void openUrl(href).catch(() => window.open(href, "_blank", "noopener"))
      }}
      className="text-accent-brand-strong underline underline-offset-2 hover:opacity-80"
    >
      {children}
    </a>
  )
}
