// Session detail —— 四栏工作台中栏的「选中会话」态（定稿 §2）。标题行
// （标题 + 收藏 + 返回）+ 对话流时间线，其余统计与身份全部去右栏
// （stats-rail 的「按会话」4 卡），阅读空间最大化。
//
// The transcript is a three-voice timeline: assistant bubbles sit left, user
// bubbles right (mirrored, corner-cut toward the edge), tool / system rows
// span full width in the middle as the "workbench". 用户/系统气泡点一下收成
// 一行（默认展开）；AI 卡整体不可折叠——卡内工具列表默认收起（徽标右侧箭
// 头总开关），单工具再各自展开。Esc / ← 返回容器态（onClose）。
//
// Rendering + the detail-local state machines. The list / scope / paging /
// mutation wiring lives in useSessionsBrowser; this file owns what only the
// detail sheet needs: the title-rename editor state (useSessionTitleRename —
// its only consumer is SessionHeader), the per-message collapse map (lifted
// out of the rows because the virtualized list unmounts off-screen rows and
// would lose per-row state) and the turn-nav bookkeeping (useTurnNav). The
// timeline is virtualized (react-virtuoso): only the rows near the viewport
// are in the DOM, so a multi-thousand-message session stays fast no matter
// how long it grows. Virtuoso measures each row's height dynamically, so
// collapsing a bubble re-lays the list without any manual bookkeeping.

import {
  ArrowLeft,
  ChevronDown,
  ChevronsDownUp,
  ChevronsUpDown,
  ChevronUp,
  Pencil,
  Search,
  Star,
  X,
} from "lucide-react"
import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import type { VirtuosoHandle } from "react-virtuoso"
import { useSetSessionCustomTitleMutation } from "@/app/store/api"
import { FilterSelect } from "@/components/filter-select"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { formatTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import type {
  SessionGroup,
  SessionMessage,
  SessionRow,
} from "@/types/generated/bindings"
import { highlight } from "../highlight"
import { sessionSourceLabel } from "../source-labels"
import {
  collapseAllMessages,
  expandAllMessages,
  firstLine,
  isAllCollapsed,
  isRowOpen,
  transcriptMatches,
} from "../transcript"
import { initialTurnNav, reduceTurnNav, turnAnchors } from "../turn-nav"
import { initialTurnSearch, reduceTurnSearch } from "../turn-search"
import { ConversationFlow } from "./conversation-flow"

/** How far below the transcript's top a jumped-to user turn lands. */
const TURN_OFFSET = 72

/** How long the jumped-to bubble keeps flashing its ring (3 pulses, .msg-flash,
 *  plus a little slack so the animation always finishes before the state drops
 *  and the ring is never left half-drawn). */
const FLASH_MS = 1200

// Turn-nav 列恒在——它是阅读的坐标系，优先级高于统计栏。显隐与压缩全部
// 引用外层命名容器 /sessions（sessions-view 的 @container/sessions），与树/
// 右栏/统计图标同一把尺：60rem 以下收成纯编号窄条（序号仍可点跳转、悬停
// 仍有全文 tooltip；60 档与「半屏/小窗」手感对齐，由用户标定），60rem 起为
// 完整目录（编号+首行，w-56）；76rem 右栏才上台——树（13）+ 右栏（16）+
// 完整导航（14）+ 详情最小（26）＋间隙 ≈71.25rem 并存且留余量的宽度，低于
// 它统计走 hover 图标。类名一律源码字面量（Tailwind 扫描器只认字面量）。

export interface SessionDetailProps {
  session: SessionRow
  favorited: boolean
  onClose: () => void
  onToggleFavorite: () => void
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
  /** 窄容器的统计浮卡入口（NarrowStatsTrigger），渲染进标题行右侧操作排；
   *  宽容器右栏本体常驻，此件自身隐身。 */
  statsSlot?: ReactNode
}

export function SessionDetail(props: SessionDetailProps) {
  const {
    session: s,
    favorited,
    onClose,
    onToggleFavorite,
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
    statsSlot,
  } = props
  const turnNav = useTurnNav(transcript)
  // Esc = 返回列表。详情内更里层的 Esc 语义（重命名取消、轮次搜索退出）由
  // 各自输入框 stopPropagation 截住，不冒泡到这里；弹层类组件（分组下拉）
  // 的 Esc 走 defaultPrevented 让路。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.key !== "Escape") return
      onClose()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [onClose])
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
  // Row open-state is "in the set" xor "defaults to collapsed" — the same xor
  // rule the bulk toggle's end states come from (transcript.ts, tested), so
  // both sides share the one predicate.
  const isOpen = useCallback(
    (uuid: string, role: string) => isRowOpen(uuid, role, collapsed),
    [collapsed],
  )
  // Bulk collapse / expand — the end-state sets come from transcript (the
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
    <div className="flex min-h-0 min-w-0 flex-1 gap-3">
      <Card
        // 钻入态详情卡：档案头（border-b）+ 对话流各占上下，gap-0 py-0 推掉
        // Card 基类的节距（header 自带 p-4，transcript 行自管 padding）。
        // min-w 兜底：容器再窄（横屏最小窗口）也保持正文可读；下限同时被
        // min(…,100%) 钳在自身可用宽内，极窄时不再溢出到栏外（右栏在根
        // 容器 58rem 以下已退位，见 stats-rail）。
        className="flex min-h-0 min-w-[min(26rem,100%)] flex-1 gap-0 py-0"
      >
        {/* Header: 标题行（定稿 §2「详情头瘦身只留标题行」）——返回 + 标题
            （就地重命名）+ 收藏 + 分组归属 + 来源徽章；身份与统计全部在右栏
            「按会话」卡组。 */}
        <SessionHeader
          session={s}
          favorited={favorited}
          onBack={onClose}
          onToggleFavorite={onToggleFavorite}
          trackGroups={trackGroups}
          currentGroupId={currentGroupId}
          onSetGroup={onSetGroup}
          statsSlot={statsSlot}
        />

        {/* Body: transcript timeline */}
        <ConversationFlow
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
      </Card>
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
    </div>
  )
}

/** Title-rename 的就地状态机（架构审查Ⅳ候选⑬：从 use-sessions-browser 迁
 *  居唯一消费者同侧——此前 detail 关注点从浏览器宿主出口逃逸，本文件反向
 *  import 浏览器 hook）。管理「编辑中 / 草稿 / 提交」三态；rename mutation
 *  与 toast 策略在此自己拿（RTK hooks 全局缓存 + useMutateWithToast 每次挂
 *  载独立，无共享态）。空草稿或与现标题相同 = 放弃编辑，不落任何写入。 */
function useSessionTitleRename(session: SessionRow) {
  const [editTitle, setEditTitle] = useState(false)
  const [titleDraft, setTitleDraft] = useState("")
  const [customTitleMut] = useSetSessionCustomTitleMutation()
  const runWithToast = useMutateWithToast()

  function startEditTitle(): void {
    if (!session) return
    setEditTitle(true)
    setTitleDraft(session.title)
  }
  function cancelEditTitle(): void {
    setEditTitle(false)
  }
  async function commitEditTitle(): Promise<void> {
    if (!session) return
    const name = titleDraft.trim()
    if (!name || name === session.title) {
      setEditTitle(false)
      return
    }
    const ok = await runWithToast(
      customTitleMut,
      { id: session.id, deviceId: session.device_id, title: name },
      {
        success: { key: "sessions.toast.renamed" },
        failed: { key: "sessions.toast.failed" },
      },
    )
    if (ok) setEditTitle(false)
  }
  return {
    editTitle,
    titleDraft,
    setTitleDraft,
    startEditTitle,
    cancelEditTitle,
    commitEditTitle,
  }
}

/**
 * The detail header — ONE title row (定稿 §2). Back + renameable title on the
 * left; favorite / group assignment / source badge pinned right. Everything
 * the old dossier rows carried (identity, usage stats, models) lives in the
 * right rail's「按会话」cards now — the conversation keeps the full width's
 * reading space. The rename trigger is the title text + pencil only (w-fit),
 * a native <button> so it stays keyboard-accessible.
 */
function SessionHeader({
  session: s,
  favorited,
  onBack,
  onToggleFavorite,
  trackGroups,
  currentGroupId,
  onSetGroup,
  statsSlot,
}: {
  session: SessionRow
  favorited: boolean
  /** 返回容器态（显式出口；Esc 同一动作）。 */
  onBack: () => void
  onToggleFavorite: () => void
  trackGroups: SessionGroup[]
  currentGroupId: string
  onSetGroup: (groupId: string | null) => void
  /** 窄容器统计浮卡入口（见 SessionDetailProps.statsSlot）。 */
  statsSlot?: ReactNode
}) {
  const { t } = useTranslation()
  // Title-rename 状态就地管理（useSessionTitleRename——不再经三层 props 传递）。
  const {
    editTitle,
    titleDraft,
    setTitleDraft,
    startEditTitle,
    cancelEditTitle,
    commitEditTitle,
  } = useSessionTitleRename(s)

  return (
    <div className="border-border flex shrink-0 items-center justify-between gap-3 border-b p-3">
      <div className="flex min-w-0 flex-1 items-center gap-1">
        {/* 返回容器态 —— 显式出口（Esc 同一动作）。 */}
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={t("sessions.detail.back")}
                onClick={onBack}
                className="text-muted-foreground -ml-1 shrink-0"
              />
            }
          >
            <ArrowLeft className="size-4" />
          </TooltipTrigger>
          <TooltipContent>{t("sessions.detail.back")}</TooltipContent>
        </Tooltip>
        {editTitle ? (
          <div className="flex min-w-0 flex-1 items-center gap-1">
            <Input
              value={titleDraft}
              onChange={(e) => setTitleDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitEditTitle()
                if (e.key === "Escape") {
                  // Esc 只取消重命名，不冒泡去关详情（window 级 Esc =
                  // 返回容器态，见 SessionDetail）。
                  e.stopPropagation()
                  cancelEditTitle()
                }
              }}
              autoFocus
            />
            <Button variant="ghost" size="sm" onClick={commitEditTitle}>
              {t("common.save")}
            </Button>
            <Button variant="ghost" size="icon-sm" onClick={cancelEditTitle}>
              {t("common.cancel")}
            </Button>
          </div>
        ) : (
          <h2 className="min-w-0 flex-1 text-base font-semibold">
            <Tooltip>
              <TooltipTrigger
                render={
                  <button
                    type="button"
                    onClick={startEditTitle}
                    className="hover:text-accent-brand-strong group flex w-fit max-w-full cursor-pointer items-center gap-1.5 rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
                  />
                }
              >
                <span className="max-w-[28rem] truncate">
                  {s.title || t("sessions.untitled")}
                </span>
                <Pencil className="text-muted-foreground size-3.5 shrink-0 opacity-60 transition-opacity group-hover:opacity-100" />
              </TooltipTrigger>
              <TooltipContent>{t("sessions.detail.renameHint")}</TooltipContent>
            </Tooltip>
          </h2>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <Badge variant="secondary">{sessionSourceLabel(s.source)}</Badge>
        <Button
          variant={favorited ? "default" : "outline"}
          size="sm"
          onClick={onToggleFavorite}
        >
          <Star className={cn("size-4", favorited && "fill-current")} />
          {favorited
            ? t("sessions.row.unfavorite")
            : t("sessions.row.favorite")}
        </Button>
        <FilterSelect
          allLabel={t("sessions.detail.noGroup")}
          options={trackGroups.map((g) => ({ value: g.id, label: g.name }))}
          value={currentGroupId}
          onChange={(v) => onSetGroup(v || null)}
          // 不传 triggerSize：默认 h-8 与收藏按钮（Button sm 同为 h-8）对齐，
          // sm 档是 h-7，并排会一高一矮。
          // 分组被删后会话仍可能挂着旧 group id：不在候选里时显示「无分组」
          // 而非原值。
          fallbackLabel={t("sessions.detail.noGroup")}
        />
        {statsSlot}
      </div>
    </div>
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
  // The mode / query / hit-highlight state machine lives in the pure reducer
  // (../turn-search) — the exit semantics (toggle / Esc / clear / query change)
  // are asserted in its tests.
  const [search, setSearch] = useState(initialTurnSearch)
  const { searching, query, lastJumped } = search
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
      setSearch((s) => reduceTurnSearch(s, { type: "hit", uuid }))
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
      // 钻入态第三栏（详情右列）：恒在，仅形态随 /sessions 档位变化（60rem
      // 以下纯序号窄条，以上完整目录）——见本文件头部的阶梯注释。
      // 卡片面同标题列/详情卡（bg-card），与左栏同一节奏。
      className={cn(
        "flex w-56 shrink-0 flex-col rounded-lg border border-border bg-card p-1",
        "@max-[60rem]/sessions:w-auto @max-[60rem]/sessions:p-0.5",
      )}
    >
      {/* Toolbar: prev / next session on the left (audit walks between
        sessions), in-session search + bulk collapse / expand on the right.
        The nav icons show where each step lands; the collapse icon shows
        the ACTION it performs next (collapse when rows are open, expand
        when collapsed). Tooltips open ABOVE the buttons — top 在满高列里
        也无裁剪风险（列左缘贴详情卡，tooltip 浮层盖在其上不影响阅读）。 */}
      {/* 纯编号窄条档位下工具栏（跨会话/搜索/批量开合）让位——窄条只保留
          跳转坐标系本身；这些操作在宽档随时可用。 */}
      <div className="mb-0.5 flex shrink-0 items-center justify-between gap-1 pr-0.5 @max-[60rem]/sessions:hidden">
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
            {/* bg-tooltip! 覆盖默认的反色芯片 —— 最浮层语义面，
                与轮次行的全文预览 tooltip 一致。 */}
            <TooltipContent
              side="top"
              className="border border-border bg-tooltip! text-tooltip-foreground!"
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
              className="border border-border bg-tooltip! text-tooltip-foreground!"
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
                  onClick={() =>
                    setSearch((s) => reduceTurnSearch(s, { type: "toggle" }))
                  }
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
              className="border border-border bg-tooltip! text-tooltip-foreground!"
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
              className="border border-border bg-tooltip! text-tooltip-foreground!"
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

      {/* 行区独立滚动（工具栏钉在列顶）——满高列里长轮次列表在列内滚，
            不把整个详情行撑高。 */}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {searching ? (
          /* 搜索模式 — 全量消息里扫关键词（不只用户轮次），命中行点击
            跳转到消息本身（复用 flash 反馈）。Esc 或工具栏按钮退出。 */
          <div className="px-1 pb-1">
            <div className="relative">
              <Search className="text-muted-foreground absolute top-1/2 left-1.5 size-3 -translate-y-1/2" />
              <Input
                value={query}
                onChange={(e) =>
                  setSearch((s) =>
                    reduceTurnSearch(s, {
                      type: "query",
                      query: e.target.value,
                    }),
                  )
                }
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    // Esc 只退出搜索模式，不冒泡去关详情（window 级 Esc =
                    // 返回列表，见 SessionDetail）。
                    e.stopPropagation()
                    setSearch((s) => reduceTurnSearch(s, { type: "esc" }))
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
                        onClick={() =>
                          setSearch((s) =>
                            reduceTurnSearch(s, { type: "clear" }),
                          )
                        }
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
                    : "hover:bg-hover",
                )}
              >
                <span className="text-muted-foreground/70 font-mono text-[9px] tabular-nums">
                  {formatTime(message.ts)}
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
                          : "text-muted-foreground hover:bg-hover hover:text-foreground",
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
                  <span className="min-w-0 truncate @max-[60rem]/sessions:hidden">
                    {firstLine(turn.content) || "—"}
                  </span>
                </TooltipTrigger>
                {/* top：与工具栏按钮同向（列内无裁剪风险），轮次行靠列底
                    时向上开也不出列。 */}
                <TooltipContent
                  side="top"
                  align="start"
                  sideOffset={8}
                  className="max-h-72 max-w-md overflow-y-auto border border-border text-[13px]"
                >
                  <div className="text-muted-foreground mb-1 text-[10px] tabular-nums">
                    {formatTime(turn.ts)}
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
