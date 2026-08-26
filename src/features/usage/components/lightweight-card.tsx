// Lightweight glance card: the same main window morphs into a small,
// always-on-top, right-edge-docked "today" snapshot. Two sub-shapes (both
// docked flush-right via the Rust `dock_window_right` command — one atomic
// SetWindowPos of the OUTER rect; see lightweight-geometry.ts):
//   - tucked: a mini-bar that ALWAYS shows today's token total — the "glance"
//     value. Layout [number][→大]. The whole bar drags via startDragging() on
//     the root (a JS call, NOT data-tauri-drag-region, so it doesn't swallow the
//     number's click): a press that moves >4px starts a window drag, a press
//     that doesn't is a click → expand. →大 stops propagation so it stays a
//     pure click.
//   - expanded: a 1:1 reuse of the dashboard's TokenHero anchor, fed
//     today's filter — the "中窗口" mirrors the dashboard anchor exactly,
//     only adding a drag/title bar with expand + shrink controls.
//
// Three "windows", each reachable from the others: full ⇄ expanded ⇄ tucked,
// plus tucked → full directly via its [→大] button. Phase is store-driven
// (viewSlice.lightweightPhase); this card just renders it.
//
// Icon language (per target shape, consistent across windows): →tucked =
// AlignHorizontalJustifyEnd (a strip pinned to the right edge); →full = Airplay
// (cast to the big screen). →中 keeps PictureInPicture2 in the title bar.
//
// Button ORDER everywhere is target-size descending (大→中→小): each window
// lists its switch targets biggest-first. So the expanded title bar is
// [全→大][缩→小], not the reverse.
//
// Data: tucked reads total_tokens from a useStatsQuery scoped to "today +
// device". Expanded reuses <TokenHero filter={…}/> — which runs its own stats
// + trend queries — so the snapshot is identical to the dashboard from one
// source. Refresh is free: providers.tsx invalidates the Usage tags on every
// `usage_changed`, and the filter matches the dashboard's "today" preset.

import { Airplay, AlignHorizontalJustifyEnd, ChevronDown } from "lucide-react"
import { useEffect, useMemo, useRef } from "react"
import { useTranslation } from "react-i18next"
import { useLightweightTuck } from "@/app/shell/use-lightweight-tuck"
import { useTuckDrag } from "@/app/shell/use-tuck-drag"
import { useTuckDrawer } from "@/app/shell/use-tuck-drawer"
import {
  useDevicesQuery,
  usePreferencesQuery,
  useStatsQuery,
  ZERO_STATS,
} from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  DEFAULT_FILTER,
  type FilterState,
  patchFilter,
} from "@/app/store/slices/filterSlice"
import { setMode } from "@/app/store/slices/viewSlice"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { deviceOptionLabel } from "@/lib/device-labels"
import { formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"
import { DeviceScopeControl } from "./device-scope-control"
import { TokenHero } from "./token-hero"

/** Hover dwell before the tucked bar expands (hover-expand preference).
 *  Long enough that a mouse sweeping past the screen's right edge does not
 *  flip the glance open; short enough that a real hover feels instant. */
const HOVER_EXPAND_DELAY_MS = 250

export function LightweightCard() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const { phase, expand, tuck, setCardHeight, setTuckDrawer } =
    useLightweightTuck()
  // Whole-bar drag for the tucked mini-bar (>4px move = window drag, else a
  // plain click → expand). See use-tuck-drag.ts.
  const { armDrag, maybeDrag, disarm, dragged } = useTuckDrag()
  // Hover-to-expand is opt-in: the default is click. When hover is
  // chosen, the tucked number area also expands on mouse-enter.
  const { data: prefs } = usePreferencesQuery()
  const hoverExpand = prefs?.lightweight_expand === "hover"

  // 今日快照 · device_scope 跟随全局 (大窗口选了某设备，中/小窗今日快照也是该
  // 设备)。范围恒为"今日"——只并入设备维度，不并 model/自定义日期。filter =
  // DEFAULT_FILTER ("today" 预设, 日期在 queryFn 实时派生) + device_scope
  // 覆盖; local day 翻页或 device_scope 变更时靠 usage_changed 刷新重算。
  const deviceScope = useAppSelector((s) => s.filter.filter.device_scope)
  // 设备列表 — 仅用于 expanded 卡内设备分段的显隐 (单设备不渲染)。缓存与
  // dashboard / DeviceScopeControl 共享，无额外请求。
  const { data: devices = [] } = useDevicesQuery()
  // Hover-drawer gating + heights. Two steps: hover slides out a trigger
  // (TRIGGER_H); clicking the trigger opens the list (listH = items × row).
  const drawerEnabled = devices.length > 1 && !hoverExpand
  const TRIGGER_H = 28
  const listH = drawerEnabled ? (devices.length + 1) * 26 + 10 : 0
  // Tucked hover-drawer (two-step): hover → trigger slides out; click trigger →
  // device list. Geometry + window-height sync are passed in; the open/close
  // sequencing + 180ms anti-jitter leave live in the hook. See use-tuck-drawer.ts.
  const {
    drawerHover,
    listOpen,
    openDrawer,
    closeDrawer,
    scheduleClose,
    toggleList,
  } = useTuckDrawer({
    enabled: drawerEnabled,
    triggerH: TRIGGER_H,
    listH,
    setTuckDrawer,
    phase,
  })
  const todayFilterState = useMemo<FilterState>(
    () => ({ ...DEFAULT_FILTER, device_scope: deviceScope }),
    [deviceScope],
  )

  // tucked reads total_tokens here; expanded reuses <TokenHero> which runs its
  // own queries. ZERO_STATS keeps the first paint sane before data lands.
  const { data: stats } = useStatsQuery(todayFilterState)
  const s = stats ?? ZERO_STATS

  // ESC closes the expanded dialog-shaped card back to the full window —
  // the card declares role="dialog", so the standard dialog exit must work
  // (tucked is a 40px strip with no keyboard expectations; it keeps none).
  useEffect(() => {
    if (phase !== "expanded") return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") dispatch(setMode("full"))
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [phase, dispatch])

  // Hover-to-expand debounce: a mouse sweeping across the screen's right edge
  // would otherwise flip the tucked bar open on every pass. A 250ms dwell
  // separates "sweeping past" from "hovering".
  const hoverTimer = useRef<number | null>(null)

  // Measure the expanded card's natural height and tell the hook, so the
  // window shrinks to fit the content. Tucked is a fixed mini-bar, so skip it.
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (phase === "tucked") return
    const el = rootRef.current
    if (!el) return
    const measure = () => {
      const h = Math.ceil(el.getBoundingClientRect().height)
      if (h > 0) setCardHeight(h)
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [phase, setCardHeight])

  // Tucked mini-bar: [number] [→大]. The whole bar drags via startDragging() on
  // the root (see armDrag/maybeDrag above) — not data-tauri-drag-region, so the
  // number stays clickable. number is flex-1 (the big drag/click target); →大
  // stops propagation so a press on it never starts a drag.
  if (phase === "tucked") {
    // 本机写「本机」(与 DeviceScopeControl 一致), 对端显示 display_name。
    const items = [
      { id: "", label: t("usage.control.all") },
      ...devices.map((d) => ({
        id: d.device_id,
        label: deviceOptionLabel(d, t),
      })),
    ]
    return (
      // biome-ignore lint/a11y/noStaticElementInteractions: Tauri window drag handle + hover drawer — mouse-only startDragging with no keyboard equivalent; keyboard users reach the same actions via the inner buttons.
      <div
        onMouseDown={armDrag}
        onMouseMove={maybeDrag}
        onMouseUp={disarm}
        onMouseEnter={openDrawer}
        onMouseLeave={() => {
          disarm()
          scheduleClose()
          // Abort a pending hover-expand when the pointer leaves early.
          if (hoverTimer.current != null) {
            window.clearTimeout(hoverTimer.current)
            hoverTimer.current = null
          }
        }}
        className="bg-glance flex h-screen w-screen flex-col animate-in fade-in slide-in-from-right-2 cursor-grab overflow-hidden duration-150 motion-reduce:animate-none"
      >
        {/* h-10 = TUCKED_HEIGHT (40px): 固定占满 tucked 高度 + items-center 让
            数字/→大 垂直居中 (不再置顶)。数字条透明透出外层渐变 (用户决策
            2026-08-14: 速览卡片背景换渐变)——不再显式 bg-card 截断渐变。
            hover 展开的设备列表 drawer 保持 bg-background (灰浮层)。 */}
        <div className="relative z-10 flex h-10 shrink-0 items-center gap-1 px-1">
          <button
            type="button"
            onMouseEnter={
              hoverExpand
                ? () => {
                    if (hoverTimer.current != null) {
                      window.clearTimeout(hoverTimer.current)
                    }
                    hoverTimer.current = window.setTimeout(
                      expand,
                      HOVER_EXPAND_DELAY_MS,
                    )
                  }
                : undefined
            }
            onClick={() => {
              if (!dragged.current) expand()
            }}
            aria-label={t("usage.lightweight.expandToday")}
            className="flex flex-1 cursor-pointer items-center justify-center border-0 bg-transparent p-0"
          >
            <span className="font-semibold tabular-nums text-base leading-none">
              {formatTokens(s.total_tokens)}
            </span>
          </button>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  aria-label={t("usage.lightweight.expandFull")}
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={() => dispatch(setMode("full"))}
                  className="text-muted-foreground hover:bg-hover hover:text-foreground inline-flex w-6 shrink-0 items-center justify-center rounded-md my-0.5"
                />
              }
            >
              <Airplay className="size-3.5" />
            </TooltipTrigger>
            <TooltipContent>{t("usage.lightweight.expandFull")}</TooltipContent>
          </Tooltip>
        </div>
        {drawerHover && drawerEnabled ? (
          <button
            type="button"
            aria-label={t("usage.deviceScope.label")}
            aria-expanded={listOpen}
            onMouseDown={(e) => e.stopPropagation()}
            onClick={toggleList}
            className="border-border bg-background text-foreground hover:bg-hover flex w-full shrink-0 items-center justify-between border-t px-2 py-1 text-[11px] transition-colors"
          >
            <span className="min-w-0 flex-1 truncate">
              {deviceScope
                ? items.find((it) => it.id === deviceScope)?.label ||
                  t("common.unnamed")
                : t("usage.control.all")}
            </span>
            <ChevronDown
              className={cn(
                "text-muted-foreground size-3 shrink-0 transition-transform",
                listOpen && "rotate-180",
              )}
            />
          </button>
        ) : null}
        {drawerHover && listOpen && drawerEnabled ? (
          <fieldset
            aria-label={t("devices.currentDevice")}
            className="bg-background m-0 flex w-full min-w-0 shrink-0 flex-col gap-0.5 p-0"
          >
            {items.map((it) => {
              const selected = deviceScope === it.id
              return (
                <button
                  key={it.id || "all"}
                  type="button"
                  aria-pressed={selected}
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={() => {
                    dispatch(patchFilter({ device_scope: it.id }))
                    closeDrawer()
                  }}
                  className={cn(
                    "focus-visible:ring-ring/40 flex w-full items-center rounded-none px-2 py-1 text-[11px] outline-none transition-colors focus-visible:ring-2",
                    selected
                      ? "bg-accent-tint text-accent-brand-strong"
                      : "text-muted-foreground hover:bg-hover hover:text-foreground",
                  )}
                >
                  <span className="min-w-0 flex-1 truncate">{it.label}</span>
                </button>
              )
            })}
          </fieldset>
        ) : null}
      </div>
    )
  }

  return (
    <div
      ref={rootRef}
      role="dialog"
      aria-label={t("usage.lightweight.todayGlance")}
      className="bg-glance text-foreground lw-reveal-in flex w-screen flex-col overflow-hidden"
    >
      {/* Drag region + two actions, ordered 大→小 (biggest target first): expand
          to full, then shrink to tucked. The buttons have no
          data-tauri-drag-region so they stay clickable inside the drag bar.
          Airplay = cast to the full dashboard; AlignHorizontalJustifyEnd = the
          right-pinned mini-bar that shrink lands on. */}
      <div
        data-tauri-drag-region
        className="text-muted-foreground flex h-9 shrink-0 items-center justify-between ps-3 pe-1 text-xs select-none"
      >
        <span data-tauri-drag-region>{t("usage.lightweight.header")}</span>
        <div className="flex items-center">
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  aria-label={t("usage.lightweight.expandFull")}
                  onClick={() => dispatch(setMode("full"))}
                  className="text-muted-foreground hover:bg-hover hover:text-foreground inline-flex size-7 items-center justify-center rounded-md transition-colors"
                />
              }
            >
              <Airplay className="size-3.5" />
            </TooltipTrigger>
            <TooltipContent>{t("usage.lightweight.expandFull")}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  aria-label={t("usage.lightweight.tuck")}
                  onClick={tuck}
                  className="text-muted-foreground hover:bg-hover hover:text-foreground inline-flex size-7 items-center justify-center rounded-md transition-colors"
                />
              }
            >
              <AlignHorizontalJustifyEnd className="size-3.5" />
            </TooltipTrigger>
            <TooltipContent>{t("usage.lightweight.tuck")}</TooltipContent>
          </Tooltip>
        </div>
      </div>

      {/* The dashboard's 右中 card, unchanged. p-3 insets it off the window's
          square edge so the card's rounded corners don't sit flush against a
          square window border — the full dashboard gives the same card the
          same breathing room via the main-area padding/gap. */}
      {/* 设备视角切换 — drag-bar 下方右对齐。drag-bar 增高到 h-9(36) 后,
          selector 顶部收到 mt-2(8), 贴近 title bar、收紧上半区呼吸 (原先
          mt-3 偏空)。px-3 右缩进与 TokenHero 卡右边平齐。单设备不渲染。 */}
      {devices.length > 1 ? (
        <div className="flex justify-end px-3 mt-2">
          <DeviceScopeControl compact />
        </div>
      ) : null}
      {/* TokenHero 卡保留横向 p-3 呼吸 (圆角不贴窗口边); 上内边距收到 pt-2,
          收紧与上方 selector 行的间距。 */}
      <div className="px-3 pb-3 pt-2">
        <TokenHero filter={todayFilterState} />
      </div>
    </div>
  )
}
