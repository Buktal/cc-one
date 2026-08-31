// TurnNavPanel —— 会话详情的第三栏（详情右列）：轮次导航。一份目录式轮次
// 索引（编号 + 首行，悬停全文，点击跳转），工具栏切换成会话内搜索模式（全量
// 消息扫关键词，命中行点击落到消息本身）+ 跨会话上一条/下一条 + 批量开合。
// 从 session-detail 拆出（文件行数上限内的领域拆分——导航列是独立内聚单元，
// 只通过 props 消费详情的 transcript）。驱动它的 useTurnNav 也在这：hook 与
// 面板说的是同一坐标系（Virtuoso 范围 → active 轮次 → scrollToIndex），
// 拆就一起拆。
//
// The transcript is virtualized (react-virtuoso): where the row for a user
// turn sits is whatever Virtuoso reports through rangeChanged, and jumping
// hands the index straight to scrollToIndex — no DOM measurement, so it stays
// correct no matter how many rows are virtualized away.

import {
  ChevronDown,
  ChevronsDownUp,
  ChevronsUpDown,
  ChevronUp,
  Search,
  X,
} from "lucide-react"
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { useTranslation } from "react-i18next"
import type { VirtuosoHandle } from "react-virtuoso"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatTime } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SessionMessage } from "@/types/generated/bindings"
import { highlight } from "../highlight"
import { firstLine, transcriptMatches } from "../transcript"
import { initialTurnNav, reduceTurnNav, turnAnchors } from "../turn-nav"
import { initialTurnSearch, reduceTurnSearch } from "../turn-search"

/** How far below the transcript's top a jumped-to user turn lands. */
const TURN_OFFSET = 72

/** How long the jumped-to bubble keeps flashing its ring (3 pulses, .msg-flash,
 *  plus a little slack so the animation always finishes before the state drops
 *  and the ring is never left half-drawn). */
const FLASH_MS = 1200

/**
 * Drive the turn-nav panel from the virtualized transcript. jumpTo accepts an
 * explicit message index so the in-session search can land on assistant / tool
 * rows that are not turns. Transient interaction, deliberately local to the
 * detail view (not in useSessionsBrowser).
 */
export function useTurnNav(messages: SessionMessage[]) {
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
export function TurnNavPanel({
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
      // 以下纯序号窄条，以上完整目录）——阶梯的宽度账与档位依据见
      // session-detail 的「Turn-nav 列恒在」注释（引用外层命名容器
      // /sessions，类名一律源码字面量）。
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
