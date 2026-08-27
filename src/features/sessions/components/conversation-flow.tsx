// Conversation flow — the transcript timeline with turn structure: one turn =
// one user message through the rows before the next user message, numbered by
// a divider at each user row; tool calls attach INSIDE the AI message card
// that issued them (indented `Write · path` title rows, click to expand the
// arguments), instead of interleaving as standalone rows. A turn with no AI
// text row keeps its tool calls as a standalone dashed group.
//
// 折叠模型（嵌套开关修复后的定稿）：用户 / 系统气泡保持「点气泡=收成一行」
// 的单层手势；AI 卡整体不可折叠——卡内是两层互不隶属的独立开关：工具列表
// 总开关（数量徽标右侧的箭头按钮，默认收起）与每个工具自己的参数展开。
// 过去三层嵌套（卡→列表→单工具）点击穿透、外层误触发，结构上不再可能。
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

import {
  Bot,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Info,
  Loader2,
  User as UserIcon,
  Wrench,
} from "lucide-react"
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type RefObject,
  useMemo,
} from "react"
import { useTranslation } from "react-i18next"
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso"
import { CopyButton } from "@/components/copy-button"
import { EmptyState } from "@/components/empty-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { describeError } from "@/lib/error"
import { formatTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SessionMessage } from "@/types/generated/bindings"
import {
  conversationLayout,
  groupConversation,
  ownsFlowRow,
  toolSummary,
} from "../conversation"
import { firstLine } from "../transcript"
import { MarkdownContent, ToolContent } from "./markdown-content"

export interface ConversationFlowProps {
  messages: SessionMessage[]
  loading: boolean
  error: unknown
  onRefresh: () => void
  virtuosoRef: RefObject<VirtuosoHandle | null>
  onRangeChanged: (range: { startIndex: number }) => void
  /** Row open-state predicate (uuid × role → open), shared with the bulk
   *  collapse toggle — see transcript.ts isRowOpen for the xor rule. */
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
        itemContent={(index, m) =>
          // Padding lives on the row wrapper (not the list) so Virtuoso can
          // size each row independently; the first row carries the top padding.
          // Absorbed tool rows skip even this wrapper — returning null keeps
          // the measured height truly 0; a bare padded shell read as an
          // “empty message”.
          ownsFlowRow(m, layout) ? (
            <div className={cn("px-4 pb-2", index === 0 ? "pt-4" : "pt-2")}>
              <FlowRow
                message={m}
                layout={layout}
                isOpen={isOpen}
                onToggle={onToggle}
                flash={m.uuid === flash}
              />
            </div>
          ) : null
        }
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
  // Attached tool rows render inside their owner card; non-first members of a
  // loose group render with the group's first member.
  if (!ownsFlowRow(m, layout)) return null
  if (m.role === "tool") {
    // ownsFlowRow 保证：走到这里的工具行必是 loose 组的首行。
    const group = layout.looseGroup.get(m.uuid)
    if (!group) return null
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
          textOpen={isOpen(m.uuid, m.role)}
          // 工具列表的开合复用同一张 collapsed 集合的 xor 语义：助手 uuid 以
          // "tool" 角色读取 = 默认收起、命中即开（与单工具行同一默认，见
          // transcript.roleDefaultsCollapsed）——虚拟化卸载后状态仍在父级。
          toolsOpen={isOpen(m.uuid, "tool")}
          onToolsListToggle={() => onToggle(m.uuid)}
          flash={flash}
          isToolOpen={isOpen}
          onToolToggle={onToggle}
        />
      ) : (
        <BaseRow
          icon={m.role === "user" ? UserIcon : Info}
          tone={m.role === "user" ? "user" : "system"}
          time={m.ts}
          collapsible
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

/** One assistant message card: the body plus its attached tool calls. The
 *  card does NOT collapse as a whole — two independent layers live inside:
 *  the run-list toggle beside the count badge (default collapsed), and each
 *  tool's own argument chevron (default collapsed). 正文的全局开合只由
 *  工具栏「全部收起/展开」写入。 */
function AssistantRow({
  message: m,
  tools,
  textOpen,
  toolsOpen,
  flash,
  isToolOpen,
  onToolsListToggle,
  onToolToggle,
}: {
  message: SessionMessage
  tools: SessionMessage[]
  /** 正文的展开态（批量手势可改写；卡片本体不可再整卡点收）。 */
  textOpen: boolean
  /** 挂载工具列表的展开态（默认收起——xor 规则同单工具行）。 */
  toolsOpen: boolean
  flash: boolean
  /** 工具列表总开关：写入的是本助手 uuid（collapsed 集合，xor 同工具行）。 */
  onToolsListToggle: () => void
  onToolToggle: (uuid: string) => void
  isToolOpen: (uuid: string, role: string) => boolean
}) {
  const { t } = useTranslation()
  const listLabel = t("sessions.detail.toggleToolCalls")
  return (
    <BaseRow
      icon={Bot}
      tone="assistant"
      time={m.ts}
      model={m.model}
      copyText={m.content}
      flash={flash}
    >
      <Content text={m.content} open={textOpen} />
      {tools.length > 0 && (
        <>
          {/* 数量徽标恒显作列表锚点；右侧箭头按钮是整列的唯一总开关——
              收起即折叠成徽标 + 箭头，展开后徽标仍可见。 */}
          <div className="mt-1.5 flex items-center gap-1">
            <Badge
              variant="outline"
              className="text-muted-foreground h-4 px-1.5 font-normal text-[10px] leading-none"
            >
              {t("sessions.detail.toolCalls", { n: tools.length })}
            </Badge>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-expanded={toolsOpen}
                    aria-label={listLabel}
                    onClick={onToolsListToggle}
                  />
                }
              >
                {toolsOpen ? (
                  <ChevronUp className="size-3" />
                ) : (
                  <ChevronDown className="size-3" />
                )}
              </TooltipTrigger>
              <TooltipContent>{listLabel}</TooltipContent>
            </Tooltip>
          </div>
          {toolsOpen && (
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
          )}
        </>
      )}
    </BaseRow>
  )
}

/** One attached tool call — the `Write · path` title row with the arguments
 *  behind a click. Collapsed by default (the xor rule in transcript.ts: tool
 *  rows default collapsed); the expanded body pretty-prints the JSON input. */
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

/** Copy-to-clipboard for one message body. Constantly visible（hover 藏按钮
 *  在触屏与扫读时不可发现——定稿固定显示）; lives inside user/system rows'
 *  collapse trigger, hence stopPropagation on click. */
function MessageCopyButton({ text }: { text: string }) {
  const { t } = useTranslation()
  const label = t("sessions.detail.copyMessage")
  return (
    <Tooltip>
      <TooltipTrigger
        render={<CopyButton value={text} label={label} stopPropagation />}
      />
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

/** The chat bubble frame. Assistant floats left, user floats right (mirrored),
 *  system spans full width. User / system bubbles fold on click (collapsible);
 *  assistant cards omit that gesture entirely——其内部已有两层独立开关
 *  （列表总开关、单工具箭头），整卡再叠一层曾是点击穿透的根源。 */
function BaseRow({
  icon: Icon,
  tone,
  time,
  model,
  copyText,
  flash,
  children,
  collapsible = false,
  open = false,
  onToggle,
}: {
  icon: typeof Bot
  tone: "assistant" | "user" | "system"
  time: string
  model?: string | null
  copyText: string
  /** Ring the bubble briefly after a turn-nav jump lands on it. */
  flash?: boolean
  children: ReactNode
  /** 点气泡=收成一行预览的手势；仅单层气泡行（用户/系统）启用。 */
  collapsible?: boolean
  /** 折叠态（仅 collapsible 行有意义）。 */
  open?: boolean
  onToggle?: () => void
}) {
  const voiceClass =
    tone === "assistant"
      ? "mr-auto max-w-[min(72ch,80%)] rounded-lg rounded-bl-sm bg-muted"
      : tone === "user"
        ? "ml-auto max-w-[min(72ch,80%)] rounded-lg rounded-br-sm bg-accent-tint"
        : "bg-transparent"
  return (
    // 折叠触发器不能是 <button>：头部内嵌复制 <button>，HTML 不允许按钮嵌
    // 套；div 承担同一键盘契约（collapsible 分支动态挂载触发属性）。
    <div
      {...(collapsible
        ? {
            role: "button",
            tabIndex: 0,
            onClick: onToggle,
            onKeyDown: (e: ReactKeyboardEvent<HTMLDivElement>) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault()
                onToggle?.()
              }
            },
            "aria-expanded": open,
          }
        : {})}
      className={cn(
        "group flex px-3 py-2 text-left text-sm",
        collapsible &&
          "focus-visible:ring-ring/40 cursor-pointer focus-visible:ring-2 focus-visible:outline-none",
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
            the source file has) — formatTime 空值即「—」。 */}
            <span>{formatTime(time)}</span>
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
            {collapsible ? (
              <ChevronDown
                className={cn(
                  "size-3 transition-transform",
                  !open && "rotate-90",
                )}
              />
            ) : null}
            <MessageCopyButton text={copyText} />
          </div>
        </div>
        {children}
      </div>
    </div>
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
