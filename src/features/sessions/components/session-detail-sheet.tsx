// Session detail — two panels that mount and unmount as one unit: the big
// detail sheet (header + transcript timeline) parked right-56, and the small
// turn-nav panel at the far right — a numbered turn index with an in-session
// search mode — for quick jumps.
//
// The detail reads as a work dossier, not a chat log. Header: title
// (inline-renameable) + actions on row 1, identity (device · project · id ·
// start) on row 2, and the usage stats (requests · messages · tokens · cost ·
// duration · active · models) on row 3 — the numbers an auditor scans for are
// lifted above the labels. The transcript is a three-voice timeline: assistant
// bubbles sit left, user bubbles right (mirrored, corner-cut toward the edge),
// tool / system rows span full width in the middle as the "workbench".
// Messages collapse on click, expanded by default; tool rows collapse to their
// name, collapsed by default.
//
// Pure rendering — all state + queries live in useSessionsBrowser. The only
// local state here is transient UI interaction that does not belong in the
// hook: the per-message collapse map (lifted out of the rows because the
// virtualized list unmounts off-screen rows and would lose per-row state) and
// the turn-nav bookkeeping (useTurnNav). The timeline is virtualized
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
  Search,
  Star,
  User as UserIcon,
  Wrench,
  X,
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
  transcriptMatches,
} from "../derive"
import { highlight } from "../highlight"
import { sessionSourceLabel } from "../source-labels"
import { initialTurnNav, reduceTurnNav, turnAnchors } from "../turn-nav"
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
            onClose={onClose}
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
        messages={transcript}
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
 * The detail header — a three-row "dossier" layout. Row 1 is title + actions:
 * the renameable title on the left, favorite / group / close pinned right.
 * Row 2 is identity — device · project · id · start, where the session ran
 * and when. Row 3 is the usage stats — requests · messages · tokens · cost ·
 * duration · active · models — the numbers an auditor scans for, half-bolded
 * with tokens / cost lifted in the brand color (same emphasis as the session
 * table). Rows 2-3 are inline flows: labels muted, value following on the
 * same line, wrapping instead of stretching into equal-width columns, so a
 * short value never leaves dead space beside it. The session id leads in a
 * monospace slot with a copy button — it's the resume handle, same for every
 * source app.
 */
function SessionHeader({
  session: s,
  favorited,
  onClose,
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
  onClose: () => void
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
    <SheetHeader className="border-border flex flex-col gap-3 border-b p-4">
      {/* Row 1 — title + actions on one line */}
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
              <Tooltip>
                <TooltipTrigger
                  render={
                    <button
                      type="button"
                      onClick={onStartTitle}
                      className="hover:text-accent-brand-strong group flex w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                    />
                  }
                >
                  <span className="max-w-[24rem] truncate">
                    {s.title || t("sessions.untitled")}
                  </span>
                  <Pencil className="text-muted-foreground size-3.5 shrink-0 opacity-60 transition-opacity group-hover:opacity-100" />
                </TooltipTrigger>
                <TooltipContent>
                  {t("sessions.detail.renameHint")}
                </TooltipContent>
              </Tooltip>
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
          {/* Explicit close — the sheet's own close button is disabled
            (showClose={false}), and an auditor opens and closes sessions in
            bursts; Esc / backdrop alone is too hidden. */}
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label={t("common.close")}
                  onClick={onClose}
                  className="text-muted-foreground"
                />
              }
            >
              <X className="size-4" />
            </TooltipTrigger>
            <TooltipContent>{t("common.close")}</TooltipContent>
          </Tooltip>
        </div>
      </div>

      {/* Row 2 — 身份：在哪跑的（设备 · 项目 · ID · 开始）。窄窗口自然
        换行；项间靠间距分隔（gap-x-5），不用符号。 */}
      <div className="flex flex-wrap items-center gap-x-5 gap-y-1.5">
        <FlowItem label={t("sessions.detail.device")}>
          {deviceLabel(s.device_id)}
        </FlowItem>
        <FlowItem label={t("sessions.detail.project")}>
          <Tooltip>
            <TooltipTrigger render={<span className="max-w-56 truncate" />}>
              {s.project_dir || "—"}
            </TooltipTrigger>
            <TooltipContent>{s.project_dir || "—"}</TooltipContent>
          </Tooltip>
        </FlowItem>
        <FlowItem label={t("sessions.detail.sessionId")}>
          <span className="inline-flex min-w-0 items-center gap-1">
            <code className="font-mono">{s.id}</code>
            <CopyIdButton id={s.id} />
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.startedAt")}>
          {s.started_at ? (
            <Tooltip>
              <TooltipTrigger
                render={
                  <span className="tabular-nums">
                    {formatTime(effectiveStarted)}
                  </span>
                }
              />
              <TooltipContent>{s.started_at}</TooltipContent>
            </Tooltip>
          ) : (
            <span className="tabular-nums">{formatTime(effectiveStarted)}</span>
          )}
        </FlowItem>
      </div>

      {/* Row 3 — 统计：花了多少、多久。审计者扫视的第一目标是数字，所以
        值统一 tabular-nums + 半粗；Token 与成本是「钱」的度量，额外提为
        品牌色（与列表页同一强调规则）。模型徽章跟在统计尾部 —— 用什么
        跑的与花了多少同属「成本」语境。 */}
      <div className="flex flex-wrap items-center gap-x-5 gap-y-1.5">
        <FlowItem label={t("sessions.detail.requests")}>
          <span className="font-medium tabular-nums">
            {formatInt(s.request_count)}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.messages")}>
          <span className="font-medium tabular-nums">
            {transcriptLoading ? "—" : formatInt(transcript.length)}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.tokens")}>
          <span className="font-medium tabular-nums">
            {formatTokens(s.total_tokens)}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.cost")}>
          <span className="text-accent-brand-strong font-medium tabular-nums">
            {formatCost(s.total_cost_usd)}
          </span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.duration")}>
          <span className="tabular-nums">{spanLabel}</span>
        </FlowItem>
        <FlowItem label={t("sessions.detail.lastActive")}>
          {s.last_active_at ? (
            <Tooltip>
              <TooltipTrigger
                render={<span>{dayjs(s.last_active_at).fromNow()}</span>}
              />
              <TooltipContent>
                {dayjs(s.last_active_at).format("YYYY-MM-DD HH:mm")}
              </TooltipContent>
            </Tooltip>
          ) : (
            "—"
          )}
        </FlowItem>
        <FlowItem label={t("sessions.detail.models")}>
          {models.length === 0 ? (
            "—"
          ) : (
            <Tooltip>
              <TooltipTrigger
                render={<span className="flex flex-wrap gap-1.5" />}
              >
                {models.map((m) => (
                  <Badge
                    key={m}
                    variant="outline"
                    className="font-mono text-[10px] font-normal"
                  >
                    {m}
                  </Badge>
                ))}
              </TooltipTrigger>
              <TooltipContent>{models.join(" · ")}</TooltipContent>
            </Tooltip>
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
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={t("sessions.detail.copySessionId")}
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
            {copied ? (
              <Check className="size-3.5" />
            ) : (
              <Copy className="size-3.5" />
            )}
          </Button>
        }
      />
      <TooltipContent>{t("sessions.detail.copySessionId")}</TooltipContent>
    </Tooltip>
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
 * rows are virtualized away. jumpTo accepts an explicit message index so the
 * in-session search can land on assistant / tool rows that are not turns.
 * Transient interaction, deliberately local to the detail view (not in
 * useSessionsBrowser).
 */
function useTurnNav(messages: SessionMessage[]) {
  const virtuosoRef = useRef<VirtuosoHandle>(null)
  // User-turn anchors (index + uuid) — the coordinate rangeChanged and
  // scrollToIndex speak in, and the coordinate the routing reducer reasons
  // over. turnAnchors is the canonical, tested builder (architecture.md:
  // "测试必须跑生产路径"), so production calls it directly.
  const anchors = useMemo(() => turnAnchors(messages), [messages])
  // Routing state lives in the pure reducer (../turn-nav); the hook only adds
  // the scroll + flash side-effects on top. Reset when the transcript changes
  // — switching sessions reuses this component instance, and a stale pin from
  // the previous session (index-based) would otherwise linger.
  const [navState, setNavState] = useState(initialTurnNav)
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — drop a stale index-based pin when the transcript swaps sessions; the body needs no messages value
  useEffect(() => setNavState(initialTurnNav), [messages])
  // The bubble a turn-nav click most recently landed on, until FLASH_MS passes
  // (then it drops so the ring doesn't linger). Re-jumping resets the timer.
  const [flashUuid, setFlashUuid] = useState<string | null>(null)
  const flashTimer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(flashTimer.current), [])

  // Virtuoso reports the first visible message index; the reducer turns that
  // into the active turn (the last user turn at or above it) unless a jump pin
  // is holding — see turn-nav.ts for why a pin beats the old "skip the next
  // rangeChanged" approach.
  const onRangeChanged = useCallback(
    ({ startIndex }: { startIndex: number }) => {
      setNavState((prev) =>
        reduceTurnNav(prev, { type: "range", startIndex }, anchors),
      )
    },
    [anchors],
  )

  const jumpTo = useCallback(
    (uuid: string, index?: number) => {
      // Turn rows carry their own index; search hits pass theirs explicitly
      // (they may be assistant / tool rows, which are not user turns).
      if (index === undefined) {
        const anchor = anchors.find((a) => a.uuid === uuid)
        if (!anchor) return
        index = anchor.index
      }
      // The reducer pins the jump's owner turn (the enclosing user turn) and
      // holds it through the scroll burst that follows. The pin replaces the
      // old skip flag, which only absorbed a single rangeChanged and let the
      // burst's later events retreat the highlight to the previous turn.
      setNavState((prev) =>
        reduceTurnNav(prev, { type: "jump", targetIndex: index }, anchors),
      )
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
        index,
        align: "start",
        offset: -TURN_OFFSET,
        behavior: "auto",
      })
    },
    [anchors],
  )

  return {
    turns: anchors.map((a) => messages[a.index]),
    activeUuid: navState.activeUuid,
    flashUuid,
    jumpTo,
    virtuosoRef,
    onRangeChanged,
  }
}

/**
 * The small nav panel beside the detail sheet — a numbered turn index, like a
 * dossier's table of contents: one row per user turn, prefixed with an
 * equi-width ordinal, the row label the turn's first line (truncated to the
 * panel's ~16-char width). Clicking jumps the transcript to it; the row for
 * the turn being read stays highlighted; hovering reads the full message. A
 * search toggle swaps the panel into in-session search mode:
 * a query scans every message body (not just turns), and each hit row shows
 * its time + a highlighted snippet, clicking lands on the message itself.
 * Height follows the content (capped at the window, then it scrolls). Mounts
 * with the sheet and slides in from the same side, so the two move as one
 * unit.
 */
function TurnNavPanel({
  messages,
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
  /** Full transcript — in-session search scans it, and its uuid → index map
   *  is the jump coordinate. */
  messages: SessionMessage[]
  turns: SessionMessage[]
  activeUuid: string | null
  /** Jump to a message. Turn rows may omit the index (it lives in `turns`);
   *  search hits always pass theirs — assistant / tool rows are not turns. */
  jumpTo: (uuid: string, index?: number) => void
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
  // Panel has two modes: the numbered turn index (default) and in-session
  // search (toolbar toggle). Search hits jump to any row kind — assistant and
  // tool rows included — which is why the panel takes the full transcript.
  const [searching, setSearching] = useState(false)
  const [query, setQuery] = useState("")
  // The search hit most recently jumped to — kept highlighted until the query
  // changes, so the eye has an anchor while reading the transcript.
  const [lastJumped, setLastJumped] = useState<string | null>(null)
  // uuid → message index: the coordinate scrollToIndex speaks. Built once per
  // transcript; search hits look up through it.
  const rowIndex = useMemo(() => {
    const map = new Map<string, number>()
    for (const [i, m] of messages.entries()) map.set(m.uuid, i)
    return map
  }, [messages])
  const matches = useMemo(
    () => transcriptMatches(messages, query),
    [messages, query],
  )
  const jump = useCallback(
    (uuid: string) => {
      const index = rowIndex.get(uuid)
      if (index === undefined) return
      setLastJumped(uuid)
      jumpTo(uuid, index)
    },
    [rowIndex, jumpTo],
  )
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
          sessions), in-session search + bulk collapse / expand on the right.
          The nav icons show where each step lands; the collapse icon shows
          the ACTION it performs next (collapse when rows are open, expand
          when collapsed). Tooltips open ABOVE the buttons — the panel's left
          edge faces the sheet, so a left-side tooltip would be clipped by it
          (the previous side="left" tip never surfaced). */}
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
              {/* bg-popover! 覆盖默认的反色芯片 —— 跟随明暗模式的主题表面，
                与轮次行的全文预览 tooltip 一致。 */}
              <TooltipContent
                side="top"
                className="border border-border bg-popover! text-popover-foreground!"
              >
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
              <TooltipContent
                side="top"
                className="border border-border bg-popover! text-popover-foreground!"
              >
                {t("sessions.detail.nextSession")}
              </TooltipContent>
            </Tooltip>
          </div>
          <div className="flex items-center gap-0.5">
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-xs"
                    aria-label={t("sessions.detail.searchInSession")}
                    onClick={() => {
                      setSearching((v) => !v)
                      // Closing the mode drops the query — the panel returns
                      // to the clean turn index.
                      if (searching) {
                        setQuery("")
                        setLastJumped(null)
                      }
                    }}
                    className={cn(
                      searching && "bg-accent-tint text-accent-brand-strong",
                    )}
                  />
                }
              >
                <Search className="size-3.5" />
              </TooltipTrigger>
              <TooltipContent
                side="top"
                className="border border-border bg-popover! text-popover-foreground!"
              >
                {t("sessions.detail.searchInSession")}
              </TooltipContent>
            </Tooltip>
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
              <TooltipContent
                side="top"
                className="border border-border bg-popover! text-popover-foreground!"
              >
                {t(
                  allCollapsed
                    ? "sessions.detail.expandAll"
                    : "sessions.detail.collapseAll",
                )}
              </TooltipContent>
            </Tooltip>
          </div>
        </div>

        {searching ? (
          /* 搜索模式 — 全量消息里扫关键词（不只用户轮次），命中行点击
            跳转到消息本身（复用 flash 反馈）。Esc 或工具栏按钮退出。 */
          <div className="px-1 pb-1">
            <div className="relative">
              <Search className="text-muted-foreground absolute top-1/2 left-1.5 size-3 -translate-y-1/2" />
              <Input
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value)
                  setLastJumped(null)
                }}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    setSearching(false)
                    setQuery("")
                    setLastJumped(null)
                  }
                }}
                placeholder={t("sessions.detail.searchInSession")}
                aria-label={t("sessions.detail.searchInSession")}
                className="h-7 pr-6 pl-6 text-xs"
                autoFocus
              />
              {query ? (
                // 清空 = 退出搜索模式：空搜索态没有意义（无命中可看），
                // 一次点击直接回到干净的轮次索引。
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <button
                        type="button"
                        onClick={() => {
                          setSearching(false)
                          setQuery("")
                          setLastJumped(null)
                        }}
                        aria-label={t("sessions.detail.clearSearch")}
                        className="text-muted-foreground hover:text-foreground absolute top-1/2 right-1 -translate-y-1/2 rounded p-0.5"
                      >
                        <X className="size-3" />
                      </button>
                    }
                  />
                  <TooltipContent>
                    {t("sessions.detail.clearSearch")}
                  </TooltipContent>
                </Tooltip>
              ) : null}
            </div>
            {query.trim() ? (
              <div className="text-muted-foreground mt-1.5 px-1 text-[10px] tabular-nums">
                {matches.length > 0
                  ? t("sessions.detail.searchMatches", { n: matches.length })
                  : t("sessions.detail.searchNoMatch")}
              </div>
            ) : null}
            {matches.map(({ message, snippet }) => (
              <button
                key={message.uuid}
                type="button"
                onClick={() => jump(message.uuid)}
                className={cn(
                  "mt-0.5 flex w-full min-w-0 flex-col items-start rounded px-1.5 py-1 text-left focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none",
                  lastJumped === message.uuid
                    ? "bg-accent-tint"
                    : "hover:bg-muted",
                )}
              >
                <span className="text-muted-foreground/70 font-mono text-[9px] tabular-nums">
                  {message.ts ? dayjs(message.ts).format("MM/DD HH:mm") : "—"}
                </span>
                <span className="text-foreground w-full min-w-0 truncate text-[11px]">
                  {highlight(snippet, query)}
                </span>
              </button>
            ))}
          </div>
        ) : (
          turns.map((turn, i) => {
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
                        "flex w-full min-w-0 items-center gap-1.5 rounded py-1 pr-1.5 pl-1 text-left text-xs focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none",
                        active
                          ? "bg-accent-tint text-foreground"
                          : "text-muted-foreground hover:bg-muted hover:text-foreground",
                      )}
                    />
                  }
                >
                  {/* 轮次编号 — 目录感：w-4 固定列宽 + 右对齐（个位数与
                    两位数个位对齐），pl-1 收紧左侧留白。active 轮次升为
                    品牌色，指示「你在第几轮」。 */}
                  <span
                    className={cn(
                      "w-4 shrink-0 text-right font-mono text-[10px] tabular-nums",
                      active
                        ? "text-accent-brand-strong"
                        : "text-muted-foreground/60",
                    )}
                  >
                    {i + 1}
                  </span>
                  <span className="min-w-0 truncate">
                    {firstLine(turn.content) || "—"}
                  </span>
                </TooltipTrigger>
                <TooltipContent
                  side="left"
                  align="start"
                  sideOffset={8}
                  className="max-h-72 max-w-md overflow-y-auto border border-border text-[13px]"
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
          })
        )}
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
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            aria-label={t("sessions.detail.copyMessage")}
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
            {copied ? (
              <Check className="size-3" />
            ) : (
              <Copy className="size-3" />
            )}
          </button>
        }
      />
      <TooltipContent>{t("sessions.detail.copyMessage")}</TooltipContent>
    </Tooltip>
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
