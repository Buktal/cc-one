// 顶栏 = 导航 + 状态 + 动作 + 窗口控制一行（#105 定稿 variant-a-v2-left-nav）：
// 左簇导航（观察组 ‖ 管理组竖分隔）+ 模式徽标/设备名，中间弹性留白即拖拽区，
// 右簇新鲜度/主题/采集 + 轻量两键（→中/→小），再接平台窗口控制。侧栏与竖屏
// TopNav/StatusBar 已撤除，本行是唯一导航面。
//
// 平台窗口控制（O_DeepSeek_Desktop ADR 0003 结论）：macOS 走 tauri.conf 的
// titleBarStyle Overlay + 系统红绿灯（行内避让 84px、顶栏不画底线）；Windows/
// Linux 由 Rust builder cfg(target_os) 分支 decorations:false，此处自绘三键贴
// 右缘（46px×满高、关闭悬停 Windows 惯例红 #e81123、图标恒白）。关闭复用既有
// closeRequested 路由：appWindow.close() 触发与系统关闭相同的托盘/退出流。
//
// 整行 data-tauri-drag-region：拖拽 + 双击最大化内建，可点击子元素（按钮）
// 由 wry 按属性存在性自动豁免。恒渲染（无禁用场景）——禁用场景必须完全不
// 渲染该属性，空串/"false" 仍会触发检测（tauri#13440）。
//
// 窄窗退化（纯 CSS 媒体查询，与原型同档）：≤1360 藏设备名 / ≤1180 导航收
// 纯图标（title 补全称）/ ≤980 新鲜度文字退脉冲点 / ≤840 藏身份簇。

import { getCurrentWindow } from "@tauri-apps/api/window"
import {
  Activity,
  AlignHorizontalJustifyEnd,
  Copy,
  Gauge,
  Library,
  MessagesSquare,
  Minus,
  PictureInPicture2,
  ScrollText,
  Server,
  Settings,
  Square,
  Tags,
  X,
} from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { useAppInfoQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  setLightweightPhase,
  setMode,
  setView,
  type ViewId,
} from "@/app/store/slices/viewSlice"
import { ThemeToggle } from "@/components/theme-toggle"
import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useDeviceOptions } from "@/features/usage/use-device-options"
import { useCollectAction } from "@/hooks/use-collect-action"
import { cn } from "@/lib/utils"
import { DataFreshness } from "./data-freshness"
import { currentPlatform, topbarLayout } from "./topbar-layout"

// 7 views split into two groups — 观察 (data views) and 管理 (system config).
// The topbar renders the grouping as a vertical rule between the two runs (no
// heading text); the full label + group rides the native title so the ≤1180
// icon-only form keeps the same information on hover.
const NAV_GROUPS: Array<{
  heading: string
  items: Array<{ id: ViewId; key: string; icon: typeof Gauge; beta?: boolean }>
}> = [
  {
    heading: "nav.group.watch",
    items: [
      { id: "dashboard", key: "nav.dashboard", icon: Gauge },
      { id: "sessions", key: "nav.sessions", icon: MessagesSquare },
      { id: "logs", key: "nav.logs", icon: ScrollText },
      { id: "pricing", key: "nav.pricing", icon: Tags },
    ],
  },
  {
    heading: "nav.group.manage",
    items: [
      { id: "library", key: "nav.library", icon: Library },
      // 应用与供应商处于 beta：codex / gemini / grok / opencode 多应用接入与
      // opencode 附加模式刚上线，真实环境验证尚不充分。
      { id: "providers", key: "nav.providers", icon: Server, beta: true },
      { id: "settings", key: "nav.settings", icon: Settings },
    ],
  },
]

/** 组间的竖分隔符（观察 ‖ 管理）与右簇动作/轻量两键之间的分隔。 */
function Vsep() {
  return (
    <span aria-hidden="true" className="bg-border mx-0.5 h-4 w-px shrink-0" />
  )
}

export function TitleBar() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const view = useAppSelector((s) => s.view.view)
  const { data: info } = useAppInfoQuery()
  const synced = info?.mode === "synced"

  const layout = topbarLayout(currentPlatform())
  const modeLabel = t(synced ? "shell.synced" : "shell.standalone")
  const deviceName = info?.display_name || t("common.unnamed")
  const deviceId = info?.device_id ?? "—"

  // Collect — the single entry point (same hook the old sidebar footer used):
  // Standalone ⇒ local collect, Synced ⇒ collect + pull + push; label covers
  // the collecting × multiDevice states.
  const multiDevice = useDeviceOptions().length > 0
  const { onCollect, collecting, collectLabel } = useCollectAction(multiDevice)

  return (
    <header
      data-tauri-drag-region="true"
      className={cn(
        "bg-app text-foreground flex h-11 min-w-0 shrink-0 select-none items-center gap-2",
        layout.paddingClass,
        layout.borderClass,
      )}
    >
      {/* 左簇之一：导航（观察组 ‖ 管理组）。≤1180 收纯图标（title 补全称）。 */}
      <nav
        aria-label={t("nav.aria")}
        className="flex shrink-0 items-center gap-0.5"
      >
        {NAV_GROUPS.map((group, gi) => (
          <div key={group.heading} className="flex items-center gap-0.5">
            {gi > 0 ? <Vsep /> : null}
            {group.items.map((item) => (
              <TopNavBtn
                key={item.id}
                item={item}
                group={t(group.heading)}
                active={view === item.id}
                onClick={() => dispatch(setView(item.id))}
              />
            ))}
          </div>
        ))}
      </nav>

      {/* 左簇之二：身份状态（原 FooterDeck 身份区升格）。≤840 整簇隐藏，
          ≤1360 藏设备名（模式徽标仍在）。 */}
      <div className="flex shrink-0 items-center gap-2 max-[840px]:hidden">
        <span
          className={cn(
            "inline-flex items-center rounded-full bg-muted px-2.5 py-0.5 text-[11px] font-medium whitespace-nowrap",
            synced ? "text-primary" : "text-muted-foreground",
          )}
        >
          {modeLabel}
        </span>
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type="button"
                className="text-muted-foreground hover:bg-hover hover:text-foreground max-[1360px]:hidden rounded-md px-1.5 py-0.5 text-xs whitespace-nowrap"
              />
            }
          >
            {deviceName}
          </TooltipTrigger>
          <TooltipContent side="bottom">
            {t("shell.deviceId")} {deviceId}
          </TooltipContent>
        </Tooltip>
      </div>

      {/* 中间弹性留白：即拖拽区。wry 的 drag 检测按 mousedown target 元素本身
          的属性判定（非祖先冒泡），这个空隙 div 必须自带属性，否则落在其上
          的按下会被当成页面内容而不是拖拽。 */}
      <div className="min-w-2 flex-1" data-tauri-drag-region="true" />

      {/* 右簇：新鲜度 + 主题 + 采集主按钮 ‖ 轻量两键。 */}
      <div className="flex shrink-0 items-center gap-2">
        <DataFreshness />
        <ThemeToggle />
        <Button
          size="sm"
          className="px-3"
          disabled={collecting}
          onClick={onCollect}
        >
          <Activity className="size-3.5" />
          {collectLabel}
        </Button>
        <Vsep />
        {/* Lightweight entries: →中 (the glance card) and →小 (the docked
            mini-bar). Both enter lightweight; the phase picks which sub-shape
            lands first. Decoupled from Close — entering is not closing. */}
        <CtrlButton
          onClick={() => {
            dispatch(setMode("lightweight"))
            dispatch(setLightweightPhase("expanded"))
          }}
          label={t("titlebar.lightweight")}
          tooltip={t("titlebar.lightweight")}
        >
          <PictureInPicture2 className="size-3.5" />
        </CtrlButton>
        <CtrlButton
          onClick={() => {
            dispatch(setMode("lightweight"))
            dispatch(setLightweightPhase("tucked"))
          }}
          label={t("titlebar.lightweightSmall")}
          tooltip={t("titlebar.lightweightSmall")}
        >
          <AlignHorizontalJustifyEnd className="size-3.5" />
        </CtrlButton>
      </div>

      {/* Windows/Linux：自绘窗口控制贴右缘。 */}
      {layout.windowControls ? <WindowControls /> : null}
    </header>
  )
}

/** 顶栏导航按钮：icon + label，≤1180 收纯图标（原生 title 补「名称 · 组」
 *  全称；beta 项追加提示句——图标态下 BETA 徽标随 label 一起隐藏）。 */
function TopNavBtn({
  item,
  group,
  active,
  onClick,
}: {
  item: { id: ViewId; key: string; icon: typeof Gauge; beta?: boolean }
  group: string
  active: boolean
  onClick: () => void
}) {
  const { t } = useTranslation()
  const Icon = item.icon
  const label = t(item.key)
  const title = item.beta
    ? `${label} · ${group} — ${t("nav.betaTitle")}`
    : `${label} · ${group}`
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      title={title}
      className={cn(
        "text-muted-foreground hover:bg-hover hover:text-foreground inline-flex h-7 items-center gap-2 rounded-md px-2.5 text-[12.5px] whitespace-nowrap transition-colors",
        "max-[1180px]:size-8 max-[1180px]:justify-center max-[1180px]:px-0",
        active
          ? "bg-accent-tint font-medium text-accent-brand-strong hover:bg-accent-tint"
          : "",
      )}
    >
      <Icon className="size-[15px] shrink-0" />
      <span className="max-[1180px]:hidden">
        {label}
        {item.beta ? (
          <span className="text-accent-brand/80 ml-1.5 text-[9px] font-bold tracking-widest">
            BETA
          </span>
        ) : null}
      </span>
    </button>
  )
}

/** 自绘窗口控制三键（Windows/Linux 无边框窗口）：贴顶栏右缘、46px 宽 × 占满
 *  行高、无圆角（系统标题按钮形态，套不进 shadcn Button 的任何 variant）。
 *  最大化↔还原图标随窗口状态互切；关闭悬停用 Windows 惯例红 #e81123（图标
 *  恒白）；关闭走 close() —— closeRequested 由 Rust 侧关闭行为拦截接管（与
 *  托盘同源，尊重 close_behavior）。已接受代价：丢 Win11 Snap Layouts 悬停
 *  （tauri#4531）。 */
function WindowControls() {
  const { t } = useTranslation()
  const [maximized, setMaximized] = useState(false)

  // getCurrentWindow() is fetched lazily inside the effect / onClick handlers
  // (not in the render body), so importing / rendering this component does not
  // blow up a non-Tauri (vitest) environment — same pattern as use-tuck-drag.
  useEffect(() => {
    const appWindow = getCurrentWindow()
    void appWindow.isMaximized().then(setMaximized)
    const unlisten = appWindow.onResized(() => {
      void appWindow.isMaximized().then(setMaximized)
    })
    return () => {
      unlisten.then((u) => u())
    }
  }, [])

  const sysBtn =
    "text-muted-foreground hover:bg-muted hover:text-foreground inline-flex h-full w-[46px] shrink-0 items-center justify-center transition-colors"
  return (
    <div className="flex h-full shrink-0 self-stretch">
      <button
        type="button"
        aria-label={t("titlebar.minimize")}
        title={t("titlebar.minimize")}
        className={sysBtn}
        onClick={() => getCurrentWindow().minimize()}
      >
        <Minus className="size-3.5" />
      </button>
      <button
        type="button"
        aria-label={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
        title={maximized ? t("titlebar.restore") : t("titlebar.maximize")}
        className={sysBtn}
        onClick={() => getCurrentWindow().toggleMaximize()}
      >
        {maximized ? (
          <Copy className="size-3" />
        ) : (
          <Square className="size-3" />
        )}
      </button>
      <button
        type="button"
        aria-label={t("titlebar.close")}
        title={t("titlebar.close")}
        className={cn(sysBtn, "hover:bg-[#e81123] hover:text-white")}
        onClick={() => getCurrentWindow().close()}
      >
        <X className="size-4" />
      </button>
    </div>
  )
}

/** 轻量两键（应用内窗口形态动作，非系统 chrome）：28px 圆角小钮。 */
function CtrlButton({
  children,
  onClick,
  label,
  tooltip,
}: {
  children: ReactNode
  onClick: () => void
  label: string
  /** Hover hint — the lightweight shape switches are the only non-obvious
   *  icons in this bar (system-convention minimize/maximize/close get none). */
  tooltip?: string
}) {
  const button = (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className="text-muted-foreground hover:bg-hover hover:text-foreground inline-flex size-7 items-center justify-center rounded-md transition-colors"
    >
      {children}
    </button>
  )
  if (!tooltip) return button
  return (
    <Tooltip>
      <TooltipTrigger render={button} />
      <TooltipContent>{tooltip}</TooltipContent>
    </Tooltip>
  )
}
