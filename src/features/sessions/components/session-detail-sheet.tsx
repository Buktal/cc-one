// Session detail — two panels that mount and unmount as one unit: the big
// detail sheet (header + transcript timeline) parked right-56, and the small
// turn-nav panel at the far right listing each user message for quick jumps.
//
// Header carries the title (inline-renameable), source / project / timing /
// usage stats, the favorite star, and the move-to-group picker. The transcript
// is a three-voice timeline: assistant bubbles sit left, user bubbles right
// (mirrored, corner-cut toward the edge), tool / system rows span full width in
// the middle as the "workbench". Messages collapse on click, expanded by
// default; tool rows collapse to their name, collapsed by default.
//
// Pure rendering — all state + queries live in useSessionsBrowser. The only
// local state here is transient UI interaction that does not belong in the
// hook: the per-message collapse map (lifted out of the rows because the
// virtualized list unmounts off-screen rows and would lose per-row state) and
// the turn-nav bookkeeping (useTurnNav). The transcript is virtualized
// (react-virtuoso): only the rows near the viewport are in the DOM, so a
// multi-thousand-message session stays fast no matter how long it grows.
// Virtuoso measures each row's height dynamically, so collapsing a bubble
// re-lays the list without any manual bookkeeping.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import {
  Bot,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  ChevronUp,
  Copy,
  Info,
  Loader2,
  Pencil,
  Star,
  User as UserIcon,
  Wrench,
} from "lucide-react"
import {
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso"
import { EmptyState } from "@/components/empty-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatCost, formatInt, formatTime, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"
import type {
  SessionGroup,
  SessionMessage,
  SessionRow,
} from "@/types/generated/bindings"
import {
  collapseAllMessages,
  expandAllMessages,
  firstLine,
  isAllCollapsed,
  modelsUsed,
  sessionSpan,
} from "../derive"
import { sessionSourceLabel } from "../source-labels"
import { MarkdownContent, ToolContent } from "./markdown-content"

dayjs.extend(relativeTime)

/** A group-picker entry plus the special "no group" / "leave as-is" options.
 *  Mirrors the sidebar's ALL/UNGROUPED sentinels but the detail picker only
 *  needs "none" (clear the assignment) vs a real group. */
const NO_GROUP = "__none__"

/** How far below the transcript's top a jumped-to user turn lands. */
const TURN_OFFSET = 72

/** How long the jumped-to bubble keeps flashing its ring (3 pulses, .msg-flash,
 *  plus a little slack so the animation always finishes before the state drops
 *  and the ring is never left half-drawn). */
const FLASH_MS = 1200

/** Turn-nav panel layout. Width ≈ 16 Chinese characters; both margins are the
 *  breathing room around the small panel. The detail sheet's right offset is
 *  their sum (right margin + panel width + inter-panel gap), so the two panels
 *  tile with a gap whatever these become. Set inline because it must beat the
 *  sheet primitive's own `right-0` / `right-56` positioning. */
const NAV_PANEL_WIDTH = "14rem" // w-56
const NAV_PANEL_RIGHT = "0.75rem" // 12px from the window edge
const NAV_PANEL_GAP = "0.75rem" // 12px between the two panels

export interface SessionDetailSheetProps {
  session: SessionRow
  favorited: boolean
  onClose: () => void
  onToggleFavorite: () => void
  // title rename
  editTitle: boolean
  titleDraft: string
  onTitleDraft: (v: string) => void
  onStartTitle: () => void
  onCancelTitle: () => void
  onCommitTitle: () => void
  // group assignment
  trackGroups: SessionGroup[]
  currentGroupId: string
  onSetGroup: (groupId: string | null) => void
  // transcript
  transcript: SessionMessage[]
  transcriptLoading: boolean
  transcriptError: unknown
  onRefreshTranscript: () => void
  // prev / next session navigation (walk the visible list; page-edge steps
  // page into the adjacent page — see useSessionsBrowser.openNeighbor)
  onPrev: () => void
  onNext: () => void
  canPrev: boolean
  canNext: boolean
  // device label (for the source line)
  deviceLabel: (id: string) => string
}

export function SessionDetailSheet(props: SessionDetailSheetProps) {
  const {
    session: s,
    favorited,
    onClose,
    onToggleFavorite,
    editTitle,
    titleDraft,
    onTitleDraft,
    onStartTitle,
    onCancelTitle,
    onCommitTitle,
    trackGroups,
    currentGroupId,
    onSetGroup,
    transcript,
    transcriptLoading,
    transcriptError,
    onRefreshTranscript,
    onPrev,
    onNext,
    canPrev,
    canNext,
    deviceLabel,
  } = props
  const turnNav = useTurnNav(transcript)
  // Which rows the user has collapsed. Kept here (not per row) because the
  // virtualized list unmounts rows that scroll out of view — per-row state
  // would be lost on the way back. Messages default expanded, tool rows
  // default collapsed, so a row's open state is "in set" xor "is a tool row".
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const toggleCollapsed = useCallback((uuid: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev)
      if (next.has(uuid)) next.delete(uuid)
      else next.add(uuid)
      return next
    })
  }, [])
  const isOpen = useCallback(
    (uuid: string, role: string) =>
      role === "tool" ? collapsed.has(uuid) : !collapsed.has(uuid),
    [collapsed],
  )
  // Bulk collapse / expand — the end-state sets come from derive (the
  // membership-is-opposite-of-default rule lives there, tested). The toggle
  // flips between the two ends, so its label follows the current state.
  const allCollapsed = isAllCollapsed(transcript, collapsed)
  const toggleAll = useCallback(() => {
    setCollapsed(
      allCollapsed
        ? expandAllMessages(transcript)
        : collapseAllMessages(transcript),
    )
  }, [allCollapsed, transcript])

  return (
    <>
      <Sheet open={true} onOpenChange={(o) => !o && onClose()}>
        <SheetContent
          showClose={false}
          // Width = min(48vw, 48rem): bubbles cap at 72ch, so the old 60vw
          // left a wide dead band of empty space beside short messages on big
          // monitors — 48rem (~768px) fits a 72ch bubble plus breathing room,
          // and the narrower sheet keeps more of the list visible beside it.
          // The inline `right` parks the sheet clear of the turn-nav panel
          // plus its margins (see NAV_PANEL_*). `min-w` keeps narrow windows
          // from squeezing the transcript below a readable size;
          // `sm:max-w-none` overrides the primitive's 24rem cap.
          style={{
            right: `calc(${NAV_PANEL_RIGHT} + ${NAV_PANEL_WIDTH} + ${NAV_PANEL_GAP})`,
          }}
          className="flex w-[min(48vw,48rem)] min-w-[32rem] flex-col gap-0 overflow-hidden p-0 sm:max-w-none"
        >
          {/* Header: 会话档案 — 识别行（标题 + 操作）在上，信息流
            （身份 → 时间 → 统计 → 模型）一行在下。两行式，流式布局无
            等宽列的死区。 */}
          <SessionHeader
            session={s}
            favorited={favorited}
            editTitle={editTitle}
            titleDraft={titleDraft}
            onTitleDraft={onTitleDraft}
            onStartTitle={onStartTitle}
            onCancelTitle={onCancelTitle}
            onCommitTitle={onCommitTitle}
            onToggleFavorite={onToggleFavorite}
            trackGroups={trackGroups}
            currentGroupId={currentGroupId}
            onSetGroup={onSetGroup}
            transcript={transcript}
            transcriptLoading={transcriptLoading}
            deviceLabel={deviceLabel}
          />

          {/* Body: transcript timeline */}
          <TranscriptBody
            messages={transcript}
            loading={transcriptLoading}
            error={transcriptError}
            onRefresh={onRefreshTranscript}
            virtuosoRef={turnNav.virtuosoRef}
            onRangeChanged={turnNav.onRangeChanged}
            isOpen={isOpen}
            onToggle={toggleCollapsed}
            flashUuid={turnNav.flashUuid}
          />
        </SheetContent>
      </Sheet>
      <TurnNavPanel
        turns={turnNav.turns}
        activeUuid={turnNav.activeUuid}
        jumpTo={turnNav.jumpTo}
        allCollapsed={allCollapsed}
        onToggleAll={toggleAll}
        onPrev={onPrev}
        onNext={onNext}
        canPrev={canPrev}
        canNext={canNext}
      />
    </>
  )
}

/**
 * The detail header — a two-row "dossier" layout. Row 1 is identity + actions:
 * the renameable title on the left, favorite / group actions pinned right.
 * Row 2 is a single inline flow of facts (id · device · project · timing ·
 * usage · models), each label muted and the value following it on the same
 * line. Flow items wrap instead of stretching into equal-width columns, so a
 * short value never leaves dead space beside it — the definition grid's
 * table feel is gone and the header is one row tall. The session id leads in
 * a monospace slot with a copy button — it's the resume handle, same for
 * every source app.
 */
function SessionHeader({
  session: s,
  favorited,
  editTitle,
  titleDraft,
  onTitleDraft,
  onStartTitle,
  onCancelTitle,
  onCommitTitle,
  onToggleFavorite,
  trackGroups,
  currentGroupId,
  onSetGroup,
  transcript,
  transcriptLoading,
  deviceLabel,
}: {
  session: SessionRow
  favorited: boolean
  editTitle: boolean
  titleDraft: string
  onTitleDraft: (v: string) => void
  onStartTitle: () => void
  onCancelTitle: () => void
  onCommitTitle: () => void
  onToggleFavorite: () => void
  trackGroups: SessionGroup[]
  currentGroupId: string
  onSetGroup: (groupId: string | null) => void
  transcript: SessionMessage[]
  transcriptLoading: boolean
  deviceLabel: (id: string) => string
}) {
  const { t } = useTranslation()
  // 开始时间：sessions 表的 started_at 缺失时（旧会话常在采集时没提取到
  // 起始时间），用对话第一条消息的时间兜底 —— transcript 按 (ts, uuid)
  // 升序，第一条就是事实上的会话起点。加载中 transcript 为空，兜底自然
  // 回落到 started_at；加载完成后重算。
  const effectiveStarted = s.started_at || transcript[0]?.ts || null
  // 会话时长 = 最近活跃 − 开始（sessionSpan 把不可用的输入归一为 null）。
  const span = sessionSpan(
    effectiveStarted && s.last_active_at
      ? dayjs(s.last_active_at).diff(dayjs(effectiveStarted))
      : null,
  )
  const spanLabel = span
    ? span.days > 0
      ? t("sessions.span.daysHours", { d: span.days, h: span.hours })
      : span.hours > 0
        ? span.minutes > 0
          ? t("sessions.span.hoursMinutes", { h: span.hours, m: span.minutes })
          : t("sessions.span.hours", { h: span.hours })
        : t("sessions.span.minutes", { m: span.minutes })
    : "—"
  const models = modelsUsed(transcript)

  return (
    <SheetHeader className="border-border flex flex-col gap-3 border-b p-4 pr-10">
      {/* Row 1 — identity + actions on one line */}
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          {/* Rename trigger: only the title text + pencil icon are clickable
            (w-fit), not the rest of the row. The pencil makes the affordance
            visible; the whole button is a native <button> so it stays
            keyboard-accessible. */}
          {editTitle ? (
            <div className="flex items-center gap-1">
              <Input
                value={titleDraft}
                onChange={(e) => onTitleDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") onCommitTitle()
                  if (e.key === "Escape") onCancelTitle()
                }}
                className="h-7"
                autoFocus
              />
              <Button variant="ghost" size="sm" onClick={onCommitTitle}>
                {t("common.save")}
              </Button>
              <Button variant="ghost" size="icon-sm" onClick={onCancelTitle}>
                {t("common.cancel")}
              </Button>
            </div>
          ) : (
            <SheetTitle className="text-base">
              <button
                type="button"
                onClick={onStartTitle}
                title={t("sessions.detail.renameHint")}
                className="hover:text-accent-brand-strong group flex w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
              >
                <span className="max-w-[24rem] truncate">
                  {s.title || t("sessions.untitled")}
                </span>
                <Pencil className="text-muted-foreground size-3.5 shrink-0 opacity-60 transition-opacity group-hover:opacity-100" />
              </button>
            </SheetTitle>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Badge variant="secondary">{sessionSourceLabel(s.source)}</Badge>
          <Button
            variant={favorited ? "default" : "outline"}
            size="sm"
            className="h-7"
            onClick={onToggleFavorite}
          >
            <Star className={cn("size-4", favorited && "fill-current")} />
            {favorited
              ? t("sessions.row.unfavorite")
              : t("sessions.row.favorite")}
          </Button>
          <Select
            value={currentGroupId || NO_GROUP}
            onValueChange={(v) =>
              onSetGroup(v === NO_GROUP ? null : (v ?? null))
            }
          >
            <SelectTrigger className="h-8 w-44" size="sm">
              <SelectValue>
                {(val: string) => {
                  if (val === NO_GROUP) return t("sessions.detail.noGroup")
                  return (
                    trackGroups.find((g) => g.id === val)?.name ??
                    t("sessions.detail.noGroup")
                  )
                }}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={NO_GROUP}>
                {t("sessions.detail.noGroup")}
              </SelectItem>
              {trackGroups.map((g) => (
                <SelectItem key={g.id} value={g.id}>
                  {g.name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      </div>

      {/* Row 2 — 信息流：身份 → 时间 → 统计 → 模型 一行内联排布，窄窗口
        自然换行。与旧版 4 列等宽网格相比没有列与列之间的死区 —— 内容按
        阅读顺序流动，整块只有一行高。项间靠间距分隔（gap-x-5），不用
        符号 —— 复合格内部才是斜杠（请求/消息、Token/成本）。 */}
      <div className="flex flex-wrap items-center gap-x-5 gap-y-1.5">
        <FlowItem label={t("sessions.detail.sessionId")}>
          <span className="inline-flex min-w-0 items-center gap-1">
            <code className="font-mono">{s.id}</code>
            <CopyIdButton id={s.id} />
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.device")}>
          {deviceLabel(s.device_id)}
        </FlowItem>
        <FlowItem label={t("sessions.detail.project")}>
          <span className="max-w-56 truncate" title={s.project_dir}>
            {s.project_dir || "—"}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.startedAt")}>
          <span className="tabular-nums" title={s.started_at || undefined}>
            {formatTime(effectiveStarted)}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.duration")}>
          <span className="tabular-nums">{spanLabel}</span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.lastActive")}>
          <span
            title={
              s.last_active_at
                ? dayjs(s.last_active_at).format("YYYY-MM-DD HH:mm")
                : undefined
            }
          >
            {s.last_active_at ? dayjs(s.last_active_at).fromNow() : "—"}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.requests")}>
          <span className="tabular-nums">{formatInt(s.request_count)}</span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.messages")}>
          <span className="tabular-nums">
            {transcriptLoading ? "—" : formatInt(transcript.length)}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.tokens")}>
          <span className="tabular-nums">{formatTokens(s.total_tokens)}</span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.cost")}>
          <span className="tabular-nums">{formatCost(s.total_cost_usd)}</span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.models")}>
          {models.length === 0 ? (
            "—"
          ) : (
            <span className="flex flex-wrap gap-1.5" title={models.join(" · ")}>
              {models.map((m) => (
                <Badge
                  key={m}
                  variant="outline"
                  className="font-mono text-[10px] font-normal"
                >
                  {m}
                </Badge>
              ))}
            </span>
          )}
        </FlowItem>
      </div>
    </SheetHeader>
  )
}

/** One inline flow item: a muted micro-label followed by the value on the
 *  same line. Items flow left-to-right and wrap — no equal-width columns, so
 *  short values don't leave dead space (unlike the old definition grid). */
function FlowItem({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span className="text-muted-foreground/70 shrink-0 text-[10px] leading-none">
        {label}
      </span>
      <span className="text-foreground min-w-0 text-xs">{children}</span>
    </span>
  )
}

/** Copy the session id to the clipboard — a persistent affordance (unlike the
 *  hover-only message copy), because the id is the resume handle. */
function CopyIdButton({ id }: { id: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  return (
    <Button
      variant="ghost"
      size="icon-xs"
      aria-label={t("sessions.detail.copySessionId")}
      title={t("sessions.detail.copySessionId")}
      onClick={() => {
        void navigator.clipboard
          ?.writeText(id)
          .then(() => {
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1500)
          })
          .catch(() => {})
      }}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
    </Button>
  )
}

function TranscriptBody({
  messages,
  loading,
  error,
  onRefresh,
  virtuosoRef,
  onRangeChanged,
  isOpen,
  onToggle,
  flashUuid,
}: {
  messages: SessionMessage[]
  loading: boolean
  error: unknown
  onRefresh: () => void
  virtuosoRef: RefObject<VirtuosoHandle | null>
  onRangeChanged: (range: { startIndex: number }) => void
  isOpen: (uuid: string, role: string) => boolean
  onToggle: (uuid: string) => void
  flashUuid: string | null
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
      <div className="text-muted-foreground flex min-h-0 flex-1 items-center justify-center p-8 text-sm">
        {t("common.loadFailed", { detail: "" })}
      </div>
    )
  }
  if (messages.length === 0) {
    // Empty = the transcript isn't in the db yet. Every session's messages land
    // in `session_messages` regardless of favorite status, so this is a
    // collection lag (the next collect picks them up), not a favorite gate.
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center p-6">
        <EmptyState
          icon={Bot}
          title={t("sessions.detail.transcriptCollecting")}
          description={t("sessions.detail.transcriptCollectingHint")}
          action={{
            label: t("sessions.detail.refresh"),
            onClick: onRefresh,
          }}
        />
      </div>
    )
  }
  return (
    <Virtuoso
      ref={virtuosoRef}
      className="min-h-0 flex-1"
      data={messages}
      computeItemKey={(_, m) => m.uuid}
      rangeChanged={onRangeChanged}
      itemContent={(index, m) => (
        // Padding lives on the row wrapper (not the list) so Virtuoso can size
        // each row independently; the first row carries the top padding.
        <div className={cn("px-4 pb-2", index === 0 ? "pt-4" : "pt-2")}>
          <MessageRow
            message={m}
            open={isOpen(m.uuid, m.role)}
            onToggle={() => onToggle(m.uuid)}
            flash={m.uuid === flashUuid}
          />
        </div>
      )}
    />
  )
}

/**
 * Drive the turn-nav panel from the virtualized transcript. Where the row for
 * a user turn sits is whatever Virtuoso reports through rangeChanged (the
 * first message index in view), and jumping hands the index straight to
 * scrollToIndex — no DOM measurement, so it stays correct no matter how many
 * rows are virtualized away. Transient interaction, deliberately local to the
 * detail view (not in useSessionsBrowser).
 */
function useTurnNav(messages: SessionMessage[]) {
  const virtuosoRef = useRef<VirtuosoHandle>(null)
  // Each user turn plus its index in the full message array. rangeChanged and
  // scrollToIndex speak in message indices, so the mapping must be kept.
  const turns = useMemo(
    () =>
      messages.flatMap((m, index) =>
        m.role === "user" ? [{ index, message: m }] : [],
      ),
    [messages],
  )
  const [activeUuid, setActiveUuid] = useState<string | null>(null)
  // The bubble a turn-nav click most recently landed on, until FLASH_MS passes
  // (then it drops so the ring doesn't linger). Re-jumping resets the timer.
  const [flashUuid, setFlashUuid] = useState<string | null>(null)
  const flashTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(flashTimer.current), [])

  // Virtuoso reports the first visible message index; the active turn is the
  // last user turn at or above it — i.e. the message the user is reading near
  // the top. setState returns the same value when nothing changed, so Virtuoso
  // reporting on every scroll frame does not re-render.
  const onRangeChanged = useCallback(
    ({ startIndex }: { startIndex: number }) => {
      let active: string | null = null
      for (const t of turns) {
        if (t.index <= startIndex) active = t.message.uuid
        else break
      }
      setActiveUuid((prev) => (prev === active ? prev : active))
    },
    [turns],
  )

  const jumpTo = useCallback(
    (uuid: string) => {
      const turn = turns.find((t) => t.message.uuid === uuid)
      if (!turn) return
      // Ring the target bubble for a beat so the eye lands with the jump.
      setFlashUuid(uuid)
      window.clearTimeout(flashTimer.current)
      flashTimer.current = window.setTimeout(() => setFlashUuid(null), FLASH_MS)
      // Virtuoso adds `offset` onto the target scrollTop, so a positive value
      // shoves the row above the viewport top; a negative one parks it
      // TURN_OFFSET below it (the same landing spot as the pre-virtualization
      // measurement). `auto` (not smooth): with dynamic row heights, a smooth
      // scroll visibly over-shoots to an estimate and glides back, so the
      // ring flash carries the feedback instead.
      virtuosoRef.current?.scrollToIndex({
        index: turn.index,
        align: "start",
        offset: -TURN_OFFSET,
        behavior: "auto",
      })
    },
    [turns],
  )

  return {
    turns: turns.map((t) => t.message),
    activeUuid,
    flashUuid,
    jumpTo,
    virtuosoRef,
    onRangeChanged,
  }
}

/**
 * The small nav panel beside the detail sheet: one row per user turn, the row
 * label is the turn's first line (truncated to the panel's ~16-char width),
 * clicking jumps the transcript to it, the row for the turn currently being
 * read stays highlighted, and hovering reads the full message. Height follows
 * the message count (capped at the window, then it scrolls). Mounts with the
 * sheet and slides in from the same side, so the two move as one unit.
 */
function TurnNavPanel({
  turns,
  activeUuid,
  jumpTo,
  allCollapsed,
  onToggleAll,
  onPrev,
  onNext,
  canPrev,
  canNext,
}: {
  turns: SessionMessage[]
  activeUuid: string | null
  jumpTo: (uuid: string) => void
  /** Every message row is collapsed — the toggle then offers "expand all". */
  allCollapsed: boolean
  onToggleAll: () => void
  /** Prev / next session in the visible list (page-edge steps page over). */
  onPrev: () => void
  onNext: () => void
  canPrev: boolean
  canNext: boolean
}) {
  const { t } = useTranslation()
  // Keep the highlighted row visible when the panel scrolls internally — on a
  // long session the turn being read could otherwise sit off-screen. `nearest`
  // only scrolls when the row is actually out of view, so it never fights the
  // user's own scrolling.
  const activeRef = useRef<HTMLButtonElement>(null)
  useLayoutEffect(() => {
    if (!activeUuid) return
    activeRef.current?.scrollIntoView({ block: "nearest" })
  }, [activeUuid])
  if (turns.length === 0) return null
  return (
    <nav
      aria-label={t("sessions.detail.turnNav")}
      className="animate-in fixed top-1/2 z-[60] -translate-y-1/2 slide-in-from-right duration-200"
      style={{ right: NAV_PANEL_RIGHT, width: NAV_PANEL_WIDTH }}
    >
      <div className="max-h-[calc(100vh-4rem)] overflow-y-auto rounded-lg border border-border bg-popover p-1 shadow-lg">
        {/* Toolbar: prev / next session on the left (audit walks between
          sessions), bulk collapse / expand on the right. The nav icons show
          where each step lands; the collapse icon shows the ACTION it
          performs next (collapse when rows are open, expand when collapsed). */}
        <div className="mb-0.5 flex items-center justify-between gap-1 pr-0.5">
          <div className="flex items-center gap-0.5">
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    disabled={!canPrev}
                    aria-label={t("sessions.detail.prevSession")}
                    onClick={onPrev}
                  />
                }
              >
                <ChevronUp className="size-3.5" />
              </TooltipTrigger>
              <TooltipContent side="left">
                {t("sessions.detail.prevSession")}
              </TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    disabled={!canNext}
                    aria-label={t("sessions.detail.nextSession")}
                    onClick={onNext}
                  />
                }
              >
                <ChevronDown className="size-3.5" />
              </TooltipTrigger>
              <TooltipContent side="left">
                {t("sessions.detail.nextSession")}
              </TooltipContent>
            </Tooltip>
          </div>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-xs"
                  aria-label={t(
                    allCollapsed
                      ? "sessions.detail.expandAll"
                      : "sessions.detail.collapseAll",
                  )}
                  onClick={onToggleAll}
                />
              }
            >
              {allCollapsed ? (
                <ChevronsUpDown className="size-3.5" />
              ) : (
                <ChevronsDownUp className="size-3.5" />
              )}
            </TooltipTrigger>
            <TooltipContent side="left">
              {t(
                allCollapsed
                  ? "sessions.detail.expandAll"
                  : "sessions.detail.collapseAll",
              )}
            </TooltipContent>
          </Tooltip>
        </div>
        {turns.map((turn) => {
          const active = turn.uuid === activeUuid
          return (
            <Tooltip key={turn.uuid}>
              <TooltipTrigger
                render={
                  <button
                    type="button"
                    ref={active ? activeRef : undefined}
                    onClick={() => jumpTo(turn.uuid)}
                    className={cn(
                      "flex w-full min-w-0 items-center rounded px-1.5 py-1 text-left text-xs focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none",
                      active
                        ? "bg-accent-tint text-foreground"
                        : "text-muted-foreground hover:bg-muted hover:text-foreground",
                    )}
                  />
                }
              >
                <span className="truncate">
                  {firstLine(turn.content) || "—"}
                </span>
              </TooltipTrigger>
              <TooltipContent
                side="left"
                align="start"
                sideOffset={8}
                // Override the shared tooltip's inverted colors — a full-text
                // read needs the theme's surface, not a high-contrast chip.
                className="max-h-72 max-w-md overflow-y-auto border border-border bg-popover! text-[13px] text-popover-foreground!"
              >
                <div className="text-muted-foreground mb-1 text-[10px] tabular-nums">
                  {turn.ts ? dayjs(turn.ts).format("MM/DD HH:mm") : "—"}
                </div>
                <div className="break-words whitespace-pre-wrap">
                  {turn.content}
                </div>
              </TooltipContent>
            </Tooltip>
          )
        })}
      </div>
    </nav>
  )
}

function MessageRow({
  message: m,
  open,
  onToggle,
  flash,
}: {
  message: SessionMessage
  open: boolean
  onToggle: () => void
  /** Ring the bubble briefly — set when a turn-nav click lands on this row. */
  flash?: boolean
}) {
  switch (m.role) {
    case "assistant":
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
        </BaseRow>
      )
    case "user":
      return (
        <BaseRow
          icon={UserIcon}
          tone="user"
          time={m.ts}
          open={open}
          onToggle={onToggle}
          copyText={m.content}
          flash={flash}
        >
          <Content text={m.content} open={open} />
        </BaseRow>
      )
    case "tool":
      return <ToolRow message={m} open={open} onToggle={onToggle} />
    case "system":
      return (
        <BaseRow
          icon={Info}
          tone="system"
          time={m.ts}
          open={open}
          onToggle={onToggle}
          copyText={m.content}
          flash={flash}
        >
          <Content text={m.content} open={open} />
        </BaseRow>
      )
    default:
      return null
  }
}

function ToolRow({
  message: m,
  open,
  onToggle,
}: {
  message: SessionMessage
  open: boolean
  onToggle: () => void
}) {
  // Collapsed by default — tool output is the noisy part of a transcript, so
  // it hides behind the tool name until clicked (messages stay expanded).
  const name = m.name || firstLine(m.content) || "tool"
  return (
    <div className="bg-muted/40 group rounded-md border border-dashed px-3 py-2 text-xs">
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
        <span className="min-w-0 flex-1 truncate font-mono">{name}</span>
        <ChevronRight
          className={cn(
            "size-3 shrink-0 transition-transform",
            open && "rotate-90",
          )}
        />
        <CopyButton text={m.content} />
      </div>
      {open ? <ToolContent text={m.content} /> : null}
    </div>
  )
}

/** Copy-to-clipboard for one message. Hidden until the row is hovered (or
 *  focused); shows a check for a moment after copying. Lives inside the
 *  row's collapse trigger, hence stopPropagation on click. */
function CopyButton({ text }: { text: string }) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  return (
    <button
      type="button"
      aria-label={t("sessions.detail.copyMessage")}
      title={t("sessions.detail.copyMessage")}
      onClick={(e) => {
        e.stopPropagation()
        void navigator.clipboard
          ?.writeText(text)
          .then(() => {
            setCopied(true)
            window.setTimeout(() => setCopied(false), 1500)
          })
          .catch(() => {})
      }}
      className="hover:text-foreground rounded p-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
    >
      {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
    </button>
  )
}

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
  // Voice layout: assistant floats left, user floats right (mirrored so its
  // icon faces the edge), system stays full-width in the middle. The corner
  // cut toward each edge is the chat-bubble gesture; max-w = min(72ch, 80%)
  // caps line length on wide sheets and keeps narrow windows from filling the
  // whole row (72ch alone exceeds the content width once the sheet shrinks).
  // The whole bubble is the collapse toggle; the header row is two blocks —
  // icon + time + model on the voice side, chevron + copy on the other,
  // pinned to the row's two ends by justify-between so even a narrow bubble
  // keeps them apart. The user voice flips the whole row with flex-row-reverse
  // (each block reverses internally) for a true mirror of the assistant's.
  const voiceClass =
    tone === "assistant"
      ? "mr-auto max-w-[min(72ch,80%)] rounded-lg rounded-bl-sm bg-muted/60"
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
              the source file has), so guard it like last_active_at. */}
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
              <CopyButton text={copyText} />
            </div>
          </div>
          {children}
        </div>
      </div>
    </>
  )
}

/** Message body: collapsed → one plain-text line (no markdown parse needed);
 *  expanded → the markdown renderer (raw HTML/XML escapes to text, so pasted
 *  system-reminder blocks show verbatim, never as markup). */
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
