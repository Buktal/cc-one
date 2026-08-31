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
// Rendering + the detail-local state. The list / scope / paging / mutation
// wiring lives in useSessionsBrowser; this file owns what only the detail
// sheet needs: the per-message collapse map (lifted out of the rows because
// the virtualized list unmounts off-screen rows and would lose per-row
// state). The title-rename editor is the shared inline-edit pair's bare-hook
// form (useInlineEdit owns draft/busy/success-close; 呈现是标题行的文本
// 保存/取消键而非 InlineTextEdit 的行内 ✓/✕ 版式，且 Esc 必须在此调用点
// stopPropagation 截住「window 级 Esc 关详情」), and the turn-nav column
// (panel + its bookkeeping hook) lives in ./turn-nav-panel. The timeline is
// virtualized (react-virtuoso): only the rows near the viewport are in the
// DOM, so a multi-thousand-message session stays fast no matter how long it
// grows. Virtuoso measures each row's height dynamically, so collapsing a
// bubble re-lays the list without any manual bookkeeping.

import { ArrowLeft, Pencil, Star } from "lucide-react"
import { type ReactNode, useCallback, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
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
import { useInlineEdit } from "@/hooks/use-inline-edit"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { cn } from "@/lib/utils"
import type {
  SessionGroup,
  SessionMessage,
  SessionRow,
} from "@/types/generated/bindings"
import { sessionSourceLabel } from "../source-labels"
import {
  collapseAllMessages,
  expandAllMessages,
  isAllCollapsed,
  isRowOpen,
} from "../transcript"
import { ConversationFlow } from "./conversation-flow"
import { TurnNavPanel, useTurnNav } from "./turn-nav-panel"

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
  // 标题重命名 = 收编原语（架构审查Ⅶ候选 C1：删掉手抄的 useSessionTitleRename
  // 三态机）。draft/busy/成功收起语义全归 useInlineEdit；target 存 begin 时抓
  // 的会话快照——提交拿到改名前的 id/device_id，编辑开着时切换会话也不串写。
  // 空草稿或与现标题相同 = 放弃（静默收起，不落任何写入）；busy 在途由机器
  // 挡二次提交——Enter 与保存键共用一个在途位（保存键据此禁用），此前在途
  // 双发第二份 set_session_custom_title + 双 toast 的缺口由原语关上。呈现是
  // bare-hook 形态（不走 InlineTextEdit 的行内 ✓/✕ 版式）：文本保存/取消键
  // 跟标题行，Esc 的 stopPropagation（window 级 Esc = 返回容器态，见
  // SessionDetail）留在下方 onKeyDown 调用点。mutation 与 toast 策略就地拿
  // （RTK hooks 全局缓存 + useMutateWithToast 每次挂载独立）。
  const [customTitleMut] = useSetSessionCustomTitleMutation()
  const runWithToast = useMutateWithToast()
  const rename = useInlineEdit<SessionRow>({
    commit: async (target, draft) => {
      const name = draft.trim()
      if (!name || name === target.title) return true
      return runWithToast(
        customTitleMut,
        { id: target.id, deviceId: target.device_id, title: name },
        {
          success: { key: "sessions.toast.renamed" },
          failed: { key: "sessions.toast.failed" },
        },
      )
    },
  })

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
        {rename.target !== null ? (
          <div className="flex min-w-0 flex-1 items-center gap-1">
            <Input
              value={rename.draft}
              onChange={(e) => rename.setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void rename.commit()
                if (e.key === "Escape") {
                  // Esc 只取消重命名，不冒泡去关详情（window 级 Esc =
                  // 返回容器态，见 SessionDetail）。
                  e.stopPropagation()
                  rename.cancel()
                }
              }}
              autoFocus
            />
            <Button
              variant="ghost"
              size="sm"
              disabled={rename.busy}
              onClick={() => void rename.commit()}
            >
              {t("common.save")}
            </Button>
            <Button variant="ghost" size="icon-sm" onClick={rename.cancel}>
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
                    onClick={() => rename.begin(s, s.title)}
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
