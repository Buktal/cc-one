// App shell: collapsible sidebar nav + scrollable content.
// View switching via viewSlice (no react-router); the active view is rendered
// by App. 顶栏 (CommandBar) 已移除 — 筛选/采集收敛进各 view 的 ControlCard /
// ControlBar，主题切换与折叠 toggle 统一收进左下角 footer 控制台 (
// 统一入口)，顶部仅留 logo 作品牌锚点，视图标题由导航
// 选中态表达。Sidebar collapse persists to localStorage. 左栏视觉对齐原型 v10
// (递减三色 mark / 绿字灰底选中 / 设备 pill)，main 区去掉 max-w 让看板与日志
// 在宽屏铺满贴边 (窄内容如 settings 各自内部 max-w 居中)。

import {
  Activity,
  BookText,
  Gauge,
  Library,
  List,
  MessagesSquare,
  PanelLeftClose,
  PanelLeftOpen,
  Server,
  Settings,
  Tags,
} from "lucide-react"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { useAppInfoQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { setView, type ViewId } from "@/app/store/slices/viewSlice"
import { ThemeToggle } from "@/components/theme-toggle"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useDeviceOptions } from "@/features/usage/use-device-options"
import { useCollectAction } from "@/hooks/use-collect-action"
import { usePersistedState } from "@/lib/persistence"
import { cn } from "@/lib/utils"
import { DataFreshness } from "./data-freshness"
import { TitleBar } from "./title-bar"
import { UpdateIndicator } from "./update-card"
import { useUpdateCheck } from "./use-update-check"

const NAV: Array<{
  id: ViewId
  key: string
  icon: typeof Gauge
  beta?: boolean
}> = [
  { id: "dashboard", key: "nav.dashboard", icon: Gauge },
  // 会话管理处于 beta：跨设备同步等核心链路已合入但未经过大规模真实环境验证。
  { id: "sessions", key: "nav.sessions", icon: MessagesSquare, beta: true },
  { id: "logs", key: "nav.logs", icon: List },
  { id: "pricing", key: "nav.pricing", icon: Tags },
  { id: "library", key: "nav.library", icon: Library },
  { id: "providers", key: "nav.providers", icon: Server },
  { id: "settings", key: "nav.settings", icon: Settings },
]

const COLLAPSE_KEY = "cc-one:sidebar-collapsed"

// Logo: the radial "One" mark in Claude Code terracotta — dark sidebar gets
// the dark badge, light sidebar the light one, so the mark always stands off
// its surface. Same mark as the app/tray icon (cc-one-dark / cc-one-light).
function Logo({ collapsed }: { collapsed: boolean }) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-2.5">
      <img
        src="/cc-one-dark.svg"
        alt=""
        className="hidden dark:block size-9 shrink-0"
      />
      <img
        src="/cc-one-light.svg"
        alt=""
        className="block dark:hidden size-9 shrink-0"
      />
      {collapsed ? null : (
        <div className="flex flex-col leading-tight">
          <span className="text-sm font-semibold">cc one</span>
          <span className="text-muted-foreground text-[10px]">
            {t("shell.logoSubtitle")}
          </span>
        </div>
      )}
    </div>
  )
}

function NavItem({
  item,
  active,
  collapsed,
  onClick,
  tooltipSide = "right",
  accentBar = "left",
}: {
  item: { id: ViewId; key: string; icon: typeof Gauge; beta?: boolean }
  active: boolean
  collapsed: boolean
  onClick: () => void
  /** Tooltip direction when `collapsed` (icon-only). */
  tooltipSide?: "right" | "bottom"
  /** Which edge the active accent bar sits on. Vertical sidebar = left;
   *  the portrait top nav = bottom. */
  accentBar?: "left" | "bottom"
}) {
  const { t } = useTranslation()
  const Icon = item.icon
  const label = t(item.key)
  const button = (
    <button
      type="button"
      onClick={onClick}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex items-center rounded-lg text-sm transition-colors",
        collapsed ? "size-9 justify-center" : "w-full gap-2.5 px-3 py-2",
        active
          ? cn(
              "bg-accent-tint font-medium text-accent-brand-strong",
              accentBar === "bottom"
                ? "shadow-[inset_0_-2px_0_var(--accent-brand)]"
                : "shadow-[inset_2px_0_0_var(--accent-brand)]",
            )
          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
      )}
    >
      <Icon className={cn("shrink-0", collapsed ? "size-5" : "size-4")} />
      {collapsed ? null : (
        <span className="flex min-w-0 flex-1 items-center gap-1.5">
          <span className="truncate">{label}</span>
          {item.beta ? (
            <span
              className="text-accent-brand/80 text-[9px] font-semibold tracking-wider"
              title={t("nav.betaTitle")}
            >
              BETA
            </span>
          ) : null}
        </span>
      )}
    </button>
  )
  if (!collapsed) return button
  return (
    <Tooltip>
      <TooltipTrigger render={button} />
      <TooltipContent side={tooltipSide}>
        {item.beta ? `${label} (${t("nav.beta")})` : label}
      </TooltipContent>
    </Tooltip>
  )
}

// Collapse toggle — lives in the footer control deck next to the theme toggle
// and device status, keeping the top of the sidebar a clean brand anchor
// (logo only). Mirrors ThemeToggle's icon-button + tooltip treatment.
function CollapseButton({
  collapsed,
  onClick,
}: {
  collapsed: boolean
  onClick: () => void
}) {
  const { t } = useTranslation()
  const Icon = collapsed ? PanelLeftOpen : PanelLeftClose
  const label = collapsed ? t("shell.expandMenu") : t("shell.collapseMenu")
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground"
            onClick={onClick}
            aria-label={label}
          />
        }
      >
        <Icon className="size-4" />
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  )
}

// Icon-only collect button — the collapsed sidebar and the portrait top bar
// share this compact form (the expanded sidebar uses a full-width labeled
// button instead). tooltipSide adapts to the surface it sits on.
function CollectIconButton({
  label,
  collecting,
  onCollect,
  tooltipSide = "right",
}: {
  label: string
  collecting: boolean
  onCollect: () => void
  tooltipSide?: "right" | "bottom"
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground"
            disabled={collecting}
            onClick={onCollect}
            aria-label={label}
          />
        }
      >
        <Activity />
      </TooltipTrigger>
      <TooltipContent side={tooltipSide}>{label}</TooltipContent>
    </Tooltip>
  )
}

// Portrait (height > width) detection. The dashboard keeps the left sidebar in
// landscape but switches to a full-width top nav bar in portrait, where width
// is scarce and height isn't — 192px of sidebar would otherwise leave the
// content column cramped (the sessions table alone wants ~768px). window is
// touched only inside the effect (never at module top), so importing this
// module stays safe in a non-Tauri (vitest) environment — same pattern as
// use-tuck-drag.
function useIsPortrait() {
  // Lazy initializer so the first frame renders the right shape (no sidebar →
  // top-bar flash on a portrait launch); `typeof window` keeps this safe when
  // the component happens to render in a non-browser (vitest) environment.
  const [portrait, setPortrait] = useState(
    () =>
      typeof window !== "undefined" && window.innerHeight > window.innerWidth,
  )
  useEffect(() => {
    const update = () => setPortrait(window.innerHeight > window.innerWidth)
    update()
    window.addEventListener("resize", update)
    return () => window.removeEventListener("resize", update)
  }, [])
  return portrait
}

// Portrait navigation bar — replaces the left sidebar when the window is
// taller than it is wide. Same card surface and accent-tint selected state as
// the sidebar, but spans the full width: logo, the seven views as icon buttons
// (label on hover), then collect + theme on the right with the device-status
// dot beside them.
function TopNav({
  view,
  onNavigate,
  collecting,
  onCollect,
  collectLabel,
  synced,
  modeLabel,
  deviceName,
}: {
  view: ViewId
  onNavigate: (id: ViewId) => void
  collecting: boolean
  onCollect: () => void
  collectLabel: string
  synced: boolean
  modeLabel: string
  deviceName: string
}) {
  return (
    <div className="bg-card border-border flex h-12 shrink-0 items-center gap-1 rounded-2xl border px-2">
      <div className="px-1">
        <Logo collapsed />
      </div>
      <Separator orientation="vertical" className="mx-1 h-6" />
      <nav className="flex items-center gap-1">
        {NAV.map((item) => (
          <NavItem
            key={item.id}
            item={item}
            active={view === item.id}
            collapsed
            tooltipSide="bottom"
            accentBar="bottom"
            onClick={() => onNavigate(item.id)}
          />
        ))}
      </nav>
      <div className="ml-auto flex items-center gap-1">
        <span
          className={cn(
            "size-2 rounded-full",
            synced ? "bg-primary" : "bg-muted-foreground/40",
          )}
          title={`${modeLabel} · ${deviceName}`}
        />
        <CollectIconButton
          label={collectLabel}
          collecting={collecting}
          onCollect={onCollect}
          tooltipSide="bottom"
        />
        <ThemeToggle />
      </div>
    </div>
  )
}

// Portrait status bar — the sidebar footer's device/freshness info lands here
// once the nav moves to the top. One thin row: freshness, device, then the
// mode / changelog / version / update cluster on the right. (The full device
// id stays in the landscape sidebar footer.)
function StatusBar({
  synced,
  modeLabel,
  deviceName,
  version,
  openReleases,
}: {
  synced: boolean
  modeLabel: string
  deviceName: string
  version?: string
  openReleases: () => void
}) {
  const { t } = useTranslation()
  return (
    <div className="bg-card border-border flex h-9 shrink-0 items-center gap-3 rounded-2xl border px-3 text-xs">
      <DataFreshness />
      <span className="text-muted-foreground min-w-0 max-w-[14rem] truncate">
        {t("shell.deviceName")}
        <span className="font-medium">{deviceName}</span>
      </span>
      <div className="ml-auto flex items-center gap-2">
        <span
          className={cn(
            "inline-flex items-center rounded-full px-2.5 py-0.5 text-[11px] font-medium",
            synced ? "bg-muted text-primary" : "bg-muted text-muted-foreground",
          )}
        >
          {modeLabel}
        </span>
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type="button"
                onClick={() => void openReleases()}
                aria-label={t("shell.changelog")}
                className="text-muted-foreground hover:text-foreground inline-flex size-3.5 items-center justify-center transition-colors"
              />
            }
          >
            <BookText className="size-3.5" />
          </TooltipTrigger>
          <TooltipContent>{t("shell.changelogGithub")}</TooltipContent>
        </Tooltip>
        {version ? (
          <span className="text-muted-foreground text-[10px]">v{version}</span>
        ) : null}
        <UpdateIndicator />
      </div>
    </div>
  )
}

export function Shell({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const view = useAppSelector((s) => s.view.view)
  const { data: info } = useAppInfoQuery()
  const synced = info?.mode === "synced"
  const { openReleases } = useUpdateCheck()

  // Sidebar collapse persists across restarts (debounced write, flushed on
  // unmount — see usePersistedState). The previous raw "1"/"0" format reads
  // back truthy/falsy, so an upgrade carries the old value over transparently.
  const [collapsed, setCollapsed] = usePersistedState<boolean>(
    COLLAPSE_KEY,
    false,
  )
  const portrait = useIsPortrait()

  const modeLabel = t(synced ? "shell.synced" : "shell.standalone")
  const deviceName = info?.display_name || t("common.unnamed")

  // Collect / sync — the single entry point now that the per-view ControlBar /
  // ControlCard / sessions buttons were removed. multiDevice only tunes the
  // success-toast wording (same semantics as the old ControlCard).
  const multiDevice = useDeviceOptions().length > 0
  const { onCollect, collecting } = useCollectAction(multiDevice)
  const collectLabel = t(
    collecting
      ? multiDevice
        ? "usage.collect.syncing"
        : "usage.collect.collecting"
      : multiDevice
        ? "usage.collect.sync"
        : "usage.collect.collect",
  )

  return (
    <div className="bg-background text-foreground flex h-screen w-screen flex-col overflow-hidden">
      <TitleBar />
      {portrait ? (
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-hidden px-4 pb-4">
          <TopNav
            view={view}
            onNavigate={(id) => dispatch(setView(id))}
            collecting={collecting}
            onCollect={onCollect}
            collectLabel={collectLabel}
            synced={synced}
            modeLabel={modeLabel}
            deviceName={deviceName}
          />
          {/* min-h-0: the main is a flex item on the column's main axis
              (portrait), where min-height:auto would let tall content stretch
              it past the viewport instead of scrolling internally. */}
          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            <div className="flex-1 overflow-auto">
              {/* px-4: symmetric insets keep the content centered and clear
                  of the scrollbar (the landscape column uses pr-4 only — it
                  sits beside the sidebar, so centering is already implied).
                  min-h-full: 内容至少占满滚动容器，但允许更高——超高内容
                  （如 providers 的多卡片堆叠）由外层 overflow-auto 滚动，
                  而不是被 h-full 锁死在视口高度上裁掉。 */}
              <div className="flex min-h-full w-full flex-col px-4">
                {children}
              </div>
            </div>
          </main>
          <StatusBar
            synced={synced}
            modeLabel={modeLabel}
            deviceName={deviceName}
            version={info?.version}
            openReleases={openReleases}
          />
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 items-stretch gap-4 overflow-hidden pb-4 pl-4">
          <aside
            className={cn(
              "bg-card border-border flex shrink-0 flex-col rounded-2xl border transition-[width] duration-200",
              collapsed ? "w-16" : "w-48",
            )}
          >
            <div
              className={cn(
                "flex items-center",
                collapsed ? "justify-center px-2 py-4" : "px-4 py-4",
              )}
            >
              <Logo collapsed={collapsed} />
            </div>

            <Separator />

            {collapsed ? null : (
              <div className="text-muted-foreground/60 px-5 pt-3 pb-1 text-[10px] font-medium tracking-wide">
                {t("nav.heading")}
              </div>
            )}
            <nav
              className={cn(
                "flex flex-col gap-1.5",
                collapsed ? "items-center p-2" : "p-2",
              )}
            >
              {NAV.map((item) => (
                <NavItem
                  key={item.id}
                  item={item}
                  active={view === item.id}
                  collapsed={collapsed}
                  onClick={() => dispatch(setView(item.id))}
                />
              ))}
            </nav>

            <div className="mt-auto p-3">
              {collapsed ? (
                /* Icon-only collect — the freshness text can't fit a 16-wide
                 sidebar; the device status dot already signals online. */
                <div className="mb-3 flex flex-col items-center">
                  <CollectIconButton
                    label={collectLabel}
                    collecting={collecting}
                    onCollect={onCollect}
                  />
                </div>
              ) : (
                <div className="mb-3 flex flex-col gap-2">
                  <DataFreshness stacked />
                  <Button
                    size="sm"
                    className="w-full"
                    disabled={collecting}
                    onClick={onCollect}
                  >
                    <Activity />
                    {collectLabel}
                  </Button>
                </div>
              )}
              <Separator className="mb-3" />
              {collapsed ? (
                <div className="flex flex-col items-center">
                  <span
                    className={cn(
                      "mb-5 size-2 rounded-full",
                      synced ? "bg-primary" : "bg-muted-foreground/40",
                    )}
                    title={`${modeLabel} · ${deviceName}`}
                  />
                  <div className="flex flex-col items-center gap-2">
                    <ThemeToggle />
                    <CollapseButton
                      collapsed={collapsed}
                      onClick={() => setCollapsed((c) => !c)}
                    />
                  </div>
                </div>
              ) : (
                <div className="flex flex-col gap-1 px-1 text-xs">
                  <div className="text-muted-foreground/60 text-[10px] tracking-wide">
                    {t("shell.thisDevice")}
                  </div>
                  <div className="truncate">
                    <span className="text-muted-foreground">
                      {t("shell.deviceName")}
                    </span>
                    <span className="font-medium">{deviceName}</span>
                  </div>
                  <div className="text-muted-foreground truncate">
                    <span>{t("shell.deviceId")}</span>
                    <span className="font-mono text-[11px]">
                      {info?.device_id ?? "—"}
                    </span>
                  </div>
                  <div className="mt-1 flex items-center justify-between gap-2">
                    <span
                      className={cn(
                        "inline-flex items-center rounded-full px-2.5 py-0.5 text-[11px] font-medium",
                        synced
                          ? "bg-muted text-primary"
                          : "bg-muted text-muted-foreground",
                      )}
                    >
                      {modeLabel}
                    </span>
                    <div className="flex items-center gap-1.5">
                      <Tooltip>
                        <TooltipTrigger
                          render={
                            <button
                              type="button"
                              onClick={() => void openReleases()}
                              aria-label={t("shell.changelog")}
                              className="text-muted-foreground hover:text-foreground inline-flex size-3.5 items-center justify-center transition-colors"
                            />
                          }
                        >
                          <BookText className="size-3.5" />
                        </TooltipTrigger>
                        <TooltipContent side="right">
                          {t("shell.changelogGithub")}
                        </TooltipContent>
                      </Tooltip>
                      {info?.version ? (
                        <span className="text-muted-foreground text-[10px]">
                          v{info.version}
                        </span>
                      ) : null}
                      <UpdateIndicator />
                    </div>
                  </div>
                  <div className="mt-2 flex items-center justify-between border-border/60 border-t pt-2">
                    <ThemeToggle />
                    <CollapseButton
                      collapsed={collapsed}
                      onClick={() => setCollapsed((c) => !c)}
                    />
                  </div>
                </div>
              )}
            </div>
          </aside>

          <main className="flex min-h-0 min-w-0 flex-1 flex-col">
            <div className="flex-1 overflow-auto">
              <div className="flex min-h-full w-full flex-col pr-4">
                {children}
              </div>
            </div>
          </main>
        </div>
      )}
    </div>
  )
}
