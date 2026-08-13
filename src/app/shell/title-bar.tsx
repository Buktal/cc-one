// Custom title bar (decorations:false). A full-width drag region sits on
// top; window controls live on the right only — the left is deliberately
// empty so it never duplicates the sidebar logo. Close reuses the existing
// CloseRequested routing: appWindow.close() triggers the same
// minimize-to-tray / quit / ask flow as a system close.

import { getCurrentWindow } from "@tauri-apps/api/window"
import {
  AlignHorizontalJustifyEnd,
  Copy,
  Minus,
  PictureInPicture2,
  Square,
  X,
} from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { useAppDispatch } from "@/app/store/hooks"
import { setLightweightPhase, setMode } from "@/app/store/slices/viewSlice"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { cn } from "@/lib/utils"

export function TitleBar() {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
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

  return (
    <div
      data-tauri-drag-region
      className="flex h-9 shrink-0 select-none items-center justify-end gap-1 pe-2"
    >
      {/* Lightweight entries: →中 (the 5-field glance card) and →小
          (the docked mini-bar). Both enter lightweight; the phase picks which
          sub-shape lands first. Decoupled from Close — entering is not closing. */}
      <CtrlButton
        onClick={() => {
          dispatch(setMode("lightweight"))
          dispatch(setLightweightPhase("expanded"))
        }}
        label={t("titlebar.lightweight")}
        tooltip={t("titlebar.lightweight")}
        className="me-1"
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
        className="me-1"
      >
        <AlignHorizontalJustifyEnd className="size-3.5" />
      </CtrlButton>
      <CtrlButton
        onClick={() => getCurrentWindow().minimize()}
        label={t("titlebar.minimize")}
      >
        <Minus className="size-3.5" />
      </CtrlButton>
      <CtrlButton
        onClick={() => getCurrentWindow().toggleMaximize()}
        label={t("titlebar.maximize")}
      >
        {maximized ? (
          <Copy className="size-3.5" />
        ) : (
          <Square className="size-3.5" />
        )}
      </CtrlButton>
      <CtrlButton
        onClick={() => getCurrentWindow().close()}
        label={t("titlebar.close")}
        className="hover:bg-destructive hover:text-white"
      >
        <X className="size-3.5" />
      </CtrlButton>
    </div>
  )
}

function CtrlButton({
  children,
  onClick,
  label,
  tooltip,
  className,
}: {
  children: ReactNode
  onClick: () => void
  label: string
  /** Hover hint — the lightweight shape switches are the only non-obvious
   *  icons in this bar (system-convention minimize/maximize/close get none). */
  tooltip?: string
  className?: string
}) {
  const button = (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      className={cn(
        "text-muted-foreground hover:bg-hover hover:text-foreground inline-flex size-7 items-center justify-center rounded-md transition-colors",
        className,
      )}
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
