// 顶栏 = 导航 + 状态 + 动作 + 窗口控制一行（#105 定稿 variant-a-v2-left-nav，
// 精修：选中底线 / 状态胶囊）：
// 左簇导航（观察组 ‖ 管理组竖分隔，组间空隙拉大）→ 中间弹性留白即拖拽区
// → 右簇状态胶囊/主题/采集（进行中图标旋转） ‖ 轻量两键，再接平台窗口
// 控制。侧栏与竖屏 TopNav/StatusBar 已撤除，本行是唯一导航面。
//
// 精修两件：
//  - 选中导航项贴 bar 底的品牌色短线（浏览器 tab 语言）：Windows 态与
//    border-b 叠成「选中处底边点亮」，macOS 无底线时悬空也成立。Neutral
//    皮肤下 tint 填充（10% 灰）淡于 hover 灰底、「hover 比选中还醒目」的
//    层次倒挂由这根结构线兜底，不再动 tint 浓度。
//  - 状态胶囊（status-capsule.tsx）收拢模式徽标/设备名/新鲜度，取代原
//    左簇身份区与右簇 DataFreshness。
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
// 窄窗退化（纯 CSS 媒体查询）：≤1360 胶囊藏时间 / ≤1180 导航收纯图标
// （title 补全称）/ ≤980 胶囊藏设备名（心跳点恒显，状态永不整簇消失）。

import { getCurrentWindow } from "@tauri-apps/api/window"
import {
  Activity,
  AlignHorizontalJustifyEnd,
  Copy,
  FolderOpen,
  Gauge,
  Loader2,
  MessagesSquare,
  Minus,
  PictureInPicture2,
  Rows3,
  Server,
  Settings,
  Square,
  Tag,
  X,
} from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
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
import { useCollectAction } from "@/hooks/use-collect-action"
import { useDeviceOptions } from "@/lib/device-labels"
import { cn } from "@/lib/utils"
import { StatusCapsule } from "./status-capsule"
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
      { id: "logs", key: "nav.logs", icon: Rows3 },
      { id: "pricing", key: "nav.pricing", icon: Tag },
    ],
  },
  {
    heading: "nav.group.manage",
    items: [
      { id: "library", key: "nav.library", icon: FolderOpen },
      // 应用与供应商处于 beta：codex / gemini / grok / opencode 多应用接入与
      // opencode 附加模式刚上线，真实环境验证尚不充分。
      { id: "providers", key: "nav.providers", icon: Server, beta: true },
      { id: "settings", key: "nav.settings", icon: Settings },
    ],
  },
]

/** 竖分隔符。观察组 ‖ 管理组之间用宽档（间隙本身即分组语义，线居中再割
 *  一刀）；右簇动作 ‖ 轻量两键之间用原窄档。 */
function Vsep({ wide = false }: { wide?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn("bg-border mx-0.5 h-4 w-px shrink-0", wide && "mx-2")}
    />
  )
}

export function TitleBar() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const view = useAppSelector((s) => s.view.view)

  const layout = topbarLayout(currentPlatform())

  // Collect — the single entry point (same hook the old sidebar footer used):
  // Standalone ⇒ local collect, Synced ⇒ collect + pull + push; label covers
  // the collecting × multiDevice states.
  const multiDevice = useDeviceOptions().length > 0
  const { onCollect, collecting, collectLabel } = useCollectAction(multiDevice)

  return (
    <header
      data-tauri-drag-region="true"
      className={cn(
        "bg-app text-foreground relative flex h-11 min-w-0 shrink-0 select-none items-center gap-2",
        layout.paddingClass,
        layout.borderClass,
      )}
    >
      {/* 导航（观察组 ‖ 管理组）。≤1180 收纯图标（title 补全称）。 */}
      <nav
        aria-label={t("nav.aria")}
        className="flex h-full shrink-0 items-center gap-0.5"
      >
        {NAV_GROUPS.map((group, gi) => (
          <div key={group.heading} className="flex items-center gap-0.5">
            {gi > 0 ? <Vsep wide /> : null}
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

      {/* 中间弹性留白：即拖拽区。wry 的 drag 检测按 mousedown target 元素本身
          的属性判定（非祖先冒泡），这个空隙 div 必须自带属性，否则落在其上
          的按下会被当成页面内容而不是拖拽。 */}
      <div className="min-w-2 flex-1" data-tauri-drag-region="true" />

      {/* 右簇：状态胶囊 + 主题 + 采集主按钮 ‖ 轻量两键。 */}
      <div className="flex shrink-0 items-center gap-2">
        <StatusCapsule />
        <ThemeToggle />
        <Button
          size="sm"
          className="px-3"
          disabled={collecting}
          onClick={onCollect}
        >
          {collecting ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <Activity className="size-3.5" />
          )}
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
 *  全称；beta 项追加提示句——图标态下 BETA 徽标随 label 一起隐藏）。
 *  选中项 = tint 填充 + 品牌色文字（交互规则）＋ wrapper 贴 bar 底的品牌色
 *  短线（结构指示，见文件头注释）；wrapper 拉满行高让线落在 header 底边。 */
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
    <div className="relative flex h-full items-center">
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
      {active ? (
        <span className="bg-accent-brand absolute inset-x-1 bottom-0 h-0.5 rounded-full" />
      ) : null}
    </div>
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
