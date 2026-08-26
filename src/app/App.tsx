// Root app: providers + shell + view switch. View routing via a
// Record keyed by ViewId — adding a view is one entry, no ternary chain.
// Lightweight mode: when mode === "lightweight" the same window
// drops the Shell and renders the glance card instead; the OS window itself
// (size / always-on-top) is morphed by useWindowMode, mounted here in App.

import type { ComponentType } from "react"
import { useAppSelector } from "@/app/store/hooks"
import type { ViewId } from "@/app/store/slices/viewSlice"
import { LibraryView } from "@/features/library/components/library-view"
import { PricingView } from "@/features/pricing/components/pricing-view"
import { ProvidersView } from "@/features/providers/components/providers-view"
import { SessionsView } from "@/features/sessions/components/sessions-view"
import { SettingsView } from "@/features/settings/components/settings-view"
import { DashboardView } from "@/features/usage/components/dashboard-view"
import { LightweightCard } from "@/features/usage/components/lightweight-card"
import { LogsView } from "@/features/usage/components/logs-view"
import { Shell } from "./shell/shell"
import { useAutoTuck } from "./shell/use-auto-tuck"
import { useUpdateCheck } from "./shell/use-update-check"
import { useWindowMode } from "./shell/use-window-mode"

const VIEWS: Record<ViewId, ComponentType> = {
  dashboard: DashboardView,
  logs: LogsView,
  pricing: PricingView,
  library: LibraryView,
  sessions: SessionsView,
  providers: ProvidersView,
  settings: SettingsView,
}

/** 满高型视图（工作台类）：直挂 main 的 flex 列，高度严格 = 视口剩余
 *  空间、无外层滚动，各面板自带滚动容器（见 Shell 的 fill 注释）。 */
const FILL_VIEWS: ReadonlySet<ViewId> = new Set(["sessions"])

export default function App() {
  // Morph the OS window to match the mode. Mounted in App so it is
  // always under the Redux store, regardless of which skin renders below.
  useWindowMode()
  // Auto-tuck: an invisible full window morphs into the mini bar after the
  // configured delay (see use-auto-tuck.ts). Active only in full mode.
  useAutoTuck()
  // Startup update probe: fires once app-wide via the hook's guard,
  // regardless of full vs lightweight skin — lightweight just doesn't render
  // the indicator.
  useUpdateCheck()
  const mode = useAppSelector((s) => s.view.mode)
  const view = useAppSelector((s) => s.view.view)

  // Same window, two skins: lightweight drops the Shell entirely.
  if (mode === "lightweight") return <LightweightCard />

  const Active = VIEWS[view]
  return (
    <Shell fill={FILL_VIEWS.has(view)}>
      <Active />
    </Shell>
  )
}
