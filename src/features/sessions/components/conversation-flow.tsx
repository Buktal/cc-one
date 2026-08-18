// Conversation flow — the transcript timeline with turn structure: one turn =
// one user message through the rows before the next user message, numbered by
// a divider at each user row; tool calls attach INSIDE the AI message card
// that issued them (indented `Write · path` title rows, click to expand the
// arguments), instead of interleaving as standalone rows. A turn with no AI
// text row keeps its tool calls as a standalone dashed group.
//
// Drop-in for the detail view's transcript body: same props contract as the
// previous TranscriptBody seam (loading / error / empty states, the Virtuoso
// ref + rangeChanged wiring useTurnNav owns, per-row collapse set, flash ring).
// The virtualized list keeps the RAW message array as its data and index
// coordinate system — attached tool rows render nothing at their own index
// (zero height), so rangeChanged / scrollToIndex / computeItemKey all keep
// speaking message indices and useTurnNav works unchanged. A flash aimed at an
// attached tool row (an in-session search hit) is redirected to the assistant
// card that renders it, so the ring always lands on a visible row.

import dayjs from "dayjs"
import {
  Bot,
  ChevronDown,
  ChevronRight,
  Info,
  Loader2,
  User as UserIcon,
  Wrench,
} from "lucide-react"
import { type ReactNode, type RefObject, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso"
import { CopyButton } from "@/components/copy-button"
import { EmptyState } from "@/components/empty-state"
import { Badge } from "@/components/ui/badge"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { describeError } from "@/lib/error"
import { cn } from "@/lib/utils"
import type { SessionMessage } from "@/types/generated/bindings"
import {
  conversationLayout,
  groupConversation,
  toolSummary,
} from "../conversation"
import { firstLine } from "../derive"
import { MarkdownContent, ToolContent } from "./markdown-content"

export interface ConversationFlowProps {
  messages: SessionMessage[]
  loading: boolean
  error: unknown
  onRefresh: () => void
  virtuosoRef: RefObject<VirtuosoHandle | null>
  onRangeChanged: (range: { startIndex: number }) => void
  /** Row open-state predicate (uuid × role → open), shared with the bulk
   *  collapse toggle — see derive.ts isRowOpen for the xor rule. */
  isOpen: (uuid: string, role: string) => boolean
  onToggle: (uuid: string) => void
  flashUuid: string | null
}

export function ConversationFlow({
  messages,
  loading,
  error,
  onRefresh,
  virtuosoRef,
  onRangeChanged,
  isOpen,
  onToggle,
  flashUuid,
}: ConversationFlowProps) {
  const turns = useMemo(() => groupConversation(messages), [messages])
  const layout = useMemo(() => conversationLayout(turns), [turns])
  // A tool row's flash (search hit) redirects to the assistant card that
  // renders it — the tool's own index is a zero-height row.
  const flash = (flashUuid && layout.attachedTo.get(flashUuid)) || flashUuid

  return (
    <TranscriptStates
      messages={messages}
      loading={loading}
      error={error}
      onRefresh={onRefresh}
    >
      <Virtuoso
        ref={virtuosoRef}
        className="min-h-0 flex-1"
        data={messages}
        computeItemKey={(_, m) => m.uuid}
        rangeChanged={onRangeChanged}
        itemContent={(index, m) => (
          // Padding lives on the row wrapper (not the list) so Virtuoso can
          // size each row independently; the first row carries the top padding.
          <div className={cn("px-4 pb-2", index === 0 ? "pt-4" : "pt-2")}>
            <FlowRow
              message={m}
              layout={layout}
              isOpen={isOpen}
              onToggle={onToggle}
              flash={m.uuid === flash}
            />
          </div>
        )}
      />
    </TranscriptStates>
  )
}

/** Loading / error / empty gates around the virtualized flow — identical
 *  states to the detail view's transcript body (collect lag, retryable
 *  failure, refresh affordance). */
function TranscriptStates({
  messages,
  loading,
  error,
  onRefresh,
  children,
}: {
  messages: SessionMessage[]
  loading: boolean
  error: unknown
  onRefresh: () => void
  children: ReactNode
}) {
  const { t } = useTranslation()
  if (loading) {
    return (
      <div className="text-muted-foreground flex min-h-0 flex-1 items-center justify-center gap-2 p-8 text-sm">
        <Loader2 className="size-4 animate-spin" />
        {t("common.loading")}
      </div>
    )
  }
  if (error) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <EmptyState
          icon={Bot}
          title={t("common.loadFailed", { detail: "" })}
          description={describeError(error, t) || undefined}
          action={{ label: t("sessions.detail.refresh"), onClick: onRefresh }}
        />
      </div>
    )
  }
  if (messages.length === 0) {
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <EmptyState
          icon={Bot}
          title={t("sessions.detail.transcriptCollecting")}
          description={t("sessions.detail.transcriptCollectingHint")}
          action={{ label: t("sessions.detail.refresh"), onClick: onRefresh }}
        />
      </div>
    )
  }
  return children
}

/** One virtualized row. Attached tool rows render nothing here (they live in
 *  the assistant card); a loose group renders once at its first member's
 *  index. */
function FlowRow({
  message: m,
  layout,
  isOpen,
  onToggle,
  flash,
}: {
  message: SessionMessage
  layout: ReturnType<typeof conversationLayout>
  isOpen: (uuid: string, role: string) => boolean
  onToggle: (uuid: string) => void
  flash: boolean
}) {
  if (m.role === "tool") {
    const group = layout.looseGroup.get(m.uuid)
    // Attached rows render inside their owner card; non-first members of a
    // loose group render with the group's first member.
    if (layout.attachedTo.has(m.uuid) || !group || group[0].uuid !== m.uuid)
      return null
    return (
      <div className="space-y-1.5">
        {group.map((tool) => (
          <ToolRow
            key={tool.uuid}
            message={tool}
            open={isOpen(tool.uuid, tool.role)}
            onToggle={() => onToggle(tool.uuid)}
          />
        ))}
      </div>
    )
  }
  // Turn divider — every user row carries its turn's ordinal (the prelude and
  // non-user rows carry none).
  const turn = m.role === "user" ? layout.turnOf.get(m.uuid) : undefined
  return (
    <>
      {turn !== undefined ? <TurnDivider number={turn} /> : null}
      {m.role === "assistant" ? (
        <AssistantRow
          message={m}
          tools={layout.toolsOf.get(m.uuid) ?? []}
          open={isOpen(m.uuid, m.role)}
          onToggle={() => onToggle(m.uuid)}
          flash={flash}
          onToolToggle={onToggle}
          isToolOpen={isOpen}
        />
      ) : (
        <BaseRow
          icon={m.role === "user" ? UserIcon : Info}
          tone={m.role === "user" ? "user" : "system"}
          time={m.ts}
          open={isOpen(m.uuid, m.role)}
          onToggle={() => onToggle(m.uuid)}
          copyText={m.content}
          flash={flash}
        >
          <Content text={m.content} open={isOpen(m.uuid, m.role)} />
        </BaseRow>
      )}
    </>
  )
}

/** The `— n —` divider above each user row: a hairline with the turn's ordinal
 *  centered on it, mirroring the turn-nav panel's numbering so the two stay
 *  visually paired. */
function TurnDivider({ number }: { number: number }) {
  const { t } = useTranslation()
  return (
    // 纯视觉分隔（无焦点语义）：编号文本自身可读，不加 role。
    <div className="text-muted-foreground/60 mb-2 flex items-center gap-2.5">
      <div className="bg-border h-px min-w-6 flex-1" />
      <span className="font-mono text-[10px] tabular-nums">
        {t("sessions.detail.turnLabel", { n: number })}
      </span>
      <div className="bg-border h-px min-w-6 flex-1" />
    </div>
  )
}

/** One assistant message card: the text bubble plus its attached tool calls
 *  indented below the body (title rows always visible, details on click).
 *  Collapsed, the card keeps the one-line body preview and folds the tool list
 *  into a count badge — the bulk "collapse all" gesture stays meaningful on
 *  tool-heavy turns. */
function AssistantRow({
  message: m,
  tools,
  open,
  onToggle,
  flash,
  onToolToggle,
  isToolOpen,
}: {
  message: SessionMessage
  tools: SessionMessage[]
  open: boolean
  onToggle: () => void
  flash: boolean
  onToolToggle: (uuid: string) => void
  isToolOpen: (uuid: string, role: string) => boolean
}) {
  const { t } = useTranslation()
  return (
    <BaseRow
      icon={Bot}
      tone="assistant"
      time={m.ts}
      model={m.model}
      open={open}
      onToggle={onToggle}
      copyText={m.content}
      flash={flash}
    >
      <Content text={m.content} open={open} />
      {tools.length > 0 &&
        (open ? (
          // 内缩工具列表：左缘细线 + 缩进，视觉上「该 AI 消息做的事」。
          <div className="border-border/70 mt-2 space-y-1.5 border-l-2 pl-2.5">
            {tools.map((tool) => (
              <ToolBlock
                key={tool.uuid}
                tool={tool}
                open={isToolOpen(tool.uuid, tool.role)}
                onToggle={() => onToolToggle(tool.uuid)}
              />
            ))}
          </div>
        ) : (
          <div className="mt-1.5">
            <Badge
              variant="outline"
              className="text-muted-foreground h-4 px-1.5 font-normal text-[10px] leading-none"
            >
              {t("sessions.detail.toolCalls", { n: tools.length })}
            </Badge>
          </div>
        ))}
    </BaseRow>
  )
}

/** One attached tool call — the `Write · path` title row with the arguments
 *  behind a click. Collapsed by default (the xor rule in derive.ts: tool rows
 *  default collapsed); the expanded body pretty-prints the JSON input. */
function ToolBlock({
  tool,
  open,
  onToggle,
}: {
  tool: SessionMessage
  open: boolean
  onToggle: () => void
}) {
  const summary = toolSummary(tool.content)
  return (
    <div className="bg-muted group rounded-md border border-dashed px-2.5 py-1.5 text-xs">
      {/* biome-ignore lint/a11y/useSemanticElements: collapse trigger must not
        be a <button> — the title row embeds the copy <button>, and nested
        buttons are invalid HTML; div keeps the same keyboard contract. */}
      <div
        role="button"
        tabIndex={0}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault()
            onToggle()
          }
        }}
        aria-expanded={open}
        className="hover:text-foreground text-muted-foreground flex w-full cursor-pointer items-center gap-1.5 text-left"
      >
        <Wrench className="size-3 shrink-0" />
        <span className="shrink-0 font-mono">{tool.name || "tool"}</span>
        {summary ? (
          <>
            <span className="text-muted-foreground/50">·</span>
            <span className="min-w-0 flex-1 truncate font-mono">{summary}</span>
          </>
        ) : (
          <span className="min-w-0 flex-1" />
        )}
        <ChevronRight
          className={cn(
            "size-3 shrink-0 transition-transform",
            open && "rotate-90",
          )}
        />
        <MessageCopyButton text={tool.content} />
      </div>
      {open ? <ToolContent text={tool.content} /> : null}
    </div>
  )
}

/** A standalone tool row — the fallback for turns with no AI text row to
 *  attach to. Same shape as the attached ToolBlock, full-width. */
function ToolRow({
  message: m,
  open,
  onToggle,
}: {
  message: SessionMessage
  open: boolean
  onToggle: () => void
}) {
  const summary = toolSummary(m.content)
  const name = m.name || firstLine(m.content) || "tool"
  return (
    <div className="bg-muted group rounded-md border border-dashed px-3 py-2 text-xs">
      {/* biome-ignore lint/a11y/useSemanticElements: collapse trigger must not
        be a <button> — the header embeds the copy <button>, and nested buttons
        are invalid HTML; div keeps the same keyboard contract. */}
      <div
        role="button"
        tabIndex={0}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault()
            onToggle()
          }
        }}
        aria-expanded={open}
        className="hover:text-foreground text-muted-foreground flex w-full cursor-pointer items-center gap-1.5 text-left"
      >
        <Wrench className="size-3 shrink-0" />
        <span className="shrink-0 font-mono">{name}</span>
        {summary ? (
          <>
            <span className="text-muted-foreground/50">·</span>
            <span className="min-w-0 flex-1 truncate font-mono">{summary}</span>
          </>
        ) : (
          <span className="min-w-0 flex-1 truncate" />
        )}
        <ChevronRight
          className={cn(
            "size-3 shrink-0 transition-transform",
            open && "rotate-90",
          )}
        />
        <MessageCopyButton text={m.content} />
      </div>
      {open ? <ToolContent text={m.content} /> : null}
    </div>
  )
}

/** Copy-to-clipboard for one message body. Hidden until the row is hovered (or
 *  focused); lives inside the row's collapse trigger, hence stopPropagation on
 *  click. */
function MessageCopyButton({ text }: { text: string }) {
  const { t } = useTranslation()
  const label = t("sessions.detail.copyMessage")
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <CopyButton
            value={text}
            label={label}
            stopPropagation
            className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          />
        }
      />
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

/** The chat bubble frame. Assistant floats left, user floats right (mirrored),
 *  system spans full width; the whole bubble is the collapse toggle. Same
 *  layout contract as the detail view's rows. */
function BaseRow({
  icon: Icon,
  tone,
  time,
  model,
  open,
  onToggle,
  copyText,
  flash,
  children,
}: {
  icon: typeof Bot
  tone: "assistant" | "user" | "system"
  time: string
  model?: string | null
  open: boolean
  onToggle: () => void
  copyText: string
  /** Ring the bubble briefly after a turn-nav jump lands on it. */
  flash?: boolean
  children: ReactNode
}) {
  const voiceClass =
    tone === "assistant"
      ? "mr-auto max-w-[min(72ch,80%)] rounded-lg rounded-bl-sm bg-muted"
      : tone === "user"
        ? "ml-auto max-w-[min(72ch,80%)] rounded-lg rounded-br-sm bg-accent-tint"
        : "bg-transparent"
  return (
    <>
      {/* biome-ignore lint/a11y/useSemanticElements: collapse trigger must
        not be a <button> — the header row embeds the copy <button>, and
        nested buttons are invalid HTML; div keeps the same keyboard
        contract. */}
      <div
        role="button"
        tabIndex={0}
        onClick={onToggle}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault()
            onToggle()
          }
        }}
        aria-expanded={open}
        className={cn(
          "group focus-visible:ring-ring/40 flex cursor-pointer px-3 py-2 text-left text-sm focus-visible:ring-2 focus-visible:outline-none",
          voiceClass,
          flash && "msg-flash",
        )}
      >
        <div className="min-w-0 flex-1">
          <div
            className={cn(
              "text-muted-foreground mb-1 flex items-center justify-between gap-1.5 text-[10px]",
              tone === "user" && "flex-row-reverse",
            )}
          >
            <div
              className={cn(
                "flex items-center gap-1.5",
                tone === "user" && "flex-row-reverse",
              )}
            >
              <Icon
                className={cn(
                  "size-3.5 shrink-0",
                  tone === "system" && "text-muted-foreground/60",
                )}
              />
              {/* ts can be an empty string (codex/claude pass through whatever
              the source file has), so guard it. */}
              <span>{time ? dayjs(time).format("MM/DD HH:mm") : "—"}</span>
              {model ? (
                <Badge
                  variant="secondary"
                  className="h-4 px-1.5 font-mono text-[10px] leading-none"
                >
                  {model}
                </Badge>
              ) : null}
            </div>
            <div
              className={cn(
                "flex items-center gap-0.5",
                tone === "user" && "flex-row-reverse",
              )}
            >
              <ChevronDown
                className={cn(
                  "size-3 transition-transform",
                  !open && "rotate-90",
                )}
              />
              <MessageCopyButton text={copyText} />
            </div>
          </div>
          {children}
        </div>
      </div>
    </>
  )
}

/** Message body: collapsed → one plain-text line (no markdown parse needed);
 *  expanded → the markdown renderer (raw HTML/XML escapes to text). */
function Content({ text, open }: { text: string; open: boolean }) {
  if (!open) {
    return (
      <div className="line-clamp-1 text-left break-words whitespace-pre-wrap">
        {text}
      </div>
    )
  }
  return <MarkdownContent text={text} />
}
