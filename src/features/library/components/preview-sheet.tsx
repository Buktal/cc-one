// Preview a Library entry in the webview. Images render natively with
// Ctrl+wheel zoom; text files (json / md / txt / log) render theme-styled so
// dark mode stays dark (the browser's default iframe rendering of these is
// white — see shouldThemeRender); everything else (html / pdf / svg / unknown)
// loads in a sandboxed iframe without scripts so an uploaded HTML file cannot
// execute.

import { convertFileSrc } from "@tauri-apps/api/core"
import { Download, Loader2, Pencil } from "lucide-react"
import { useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { useLibraryTextQuery } from "@/app/store/api"
import { QueryState } from "@/components/query-state"
import { Button } from "@/components/ui/button"
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
import { cn } from "@/lib/utils"
import type { LibraryEntry } from "@/types/generated/bindings"
import { isImageName, maybePrettyJson, shouldThemeRender } from "../derive"

export function PreviewSheet({
  entry,
  busy = false,
  onClose,
  onExport,
  onRename,
}: {
  entry: LibraryEntry
  /** True while an export for this entry is in flight (row + sheet share it). */
  busy?: boolean
  onClose: () => void
  /** Optional so the sheet stays usable without actions; the toolbar only
   *  renders when a caller provides the callbacks. */
  onExport?: () => void
  onRename?: () => void
}) {
  const { t } = useTranslation()
  const url = convertFileSrc(entry.abs_path)
  const isImage = isImageName(entry.name)
  const isText = shouldThemeRender(entry.name)
  const textQuery = useLibraryTextQuery(entry.rel_path, { skip: !isText })
  const [scale, setScale] = useState(1)
  const [pos, setPos] = useState({ x: 0, y: 0 })
  const drag = useRef<{
    sx: number
    sy: number
    px: number
    py: number
  } | null>(null)

  // Ctrl+wheel zoom mirrors the Win11 / browser image habit. Once zoomed in,
  // left-drag pans — the native <img> drag is disabled (draggable=false) so a
  // left-click can't fall through to the list behind the sheet.
  function onWheel(e: React.WheelEvent) {
    if (!e.ctrlKey) return
    e.preventDefault()
    setScale((s) => Math.min(4, Math.max(0.5, s - e.deltaY * 0.002)))
  }
  function onPointerDown(e: React.PointerEvent) {
    if (scale <= 1) return
    drag.current = { sx: e.clientX, sy: e.clientY, px: pos.x, py: pos.y }
    e.currentTarget.setPointerCapture(e.pointerId)
  }
  function onPointerMove(e: React.PointerEvent) {
    if (!drag.current) return
    setPos({
      x: drag.current.px + (e.clientX - drag.current.sx),
      y: drag.current.py + (e.clientY - drag.current.sy),
    })
  }
  function onPointerUp(e: React.PointerEvent) {
    drag.current = null
    e.currentTarget.releasePointerCapture?.(e.pointerId)
  }

  return (
    <Sheet open={true} onOpenChange={(o) => !o && onClose()}>
      {/* No top-right X — a viewer shouldn't look closable-mid-content; ESC
          and the backdrop still close it. */}
      <SheetContent
        showClose={false}
        className="flex w-[640px] flex-col gap-3 sm:max-w-[640px]"
      >
        <SheetHeader className="flex flex-row items-center justify-between gap-2">
          <SheetTitle className="min-w-0 truncate">{entry.name}</SheetTitle>
          {/* Row actions, reachable without closing the preview: export keeps
              the sheet open (native folder picker on top), rename hands back
              to the row's inline editor. No delete — that goes through the
              shared confirm flow. Hidden until the caller wires callbacks. */}
          {onExport && onRename ? (
            <div className="flex shrink-0 gap-1">
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("library.row.export")}
                      disabled={busy}
                      onClick={onExport}
                    />
                  }
                >
                  {busy ? <Loader2 className="animate-spin" /> : <Download />}
                </TooltipTrigger>
                <TooltipContent>{t("library.row.export")}</TooltipContent>
              </Tooltip>
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t("library.row.rename")}
                      onClick={onRename}
                    />
                  }
                >
                  <Pencil />
                </TooltipTrigger>
                <TooltipContent>{t("library.row.rename")}</TooltipContent>
              </Tooltip>
            </div>
          ) : null}
        </SheetHeader>
        {isImage ? (
          // <img> fits the pane (max-w-full → no horizontal scroll) instead of
          // overflowing at native size inside an iframe. Ctrl+wheel zoom; once
          // zoomed, left-drag pans.
          <div
            className="border-border bg-background flex min-h-[60vh] w-full flex-1 items-center justify-center overflow-y-auto overflow-x-hidden rounded-md border p-2"
            onWheel={onWheel}
          >
            <img
              src={url}
              alt={entry.name}
              draggable={false}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              style={{
                transform: `translate(${pos.x}px, ${pos.y}px) scale(${scale})`,
              }}
              className={cn(
                "max-w-full origin-top touch-none select-none",
                scale > 1
                  ? "cursor-grab active:cursor-grabbing"
                  : "cursor-default",
              )}
            />
          </div>
        ) : isText ? (
          /* Theme-styled text preview — json pretty-printed, everything else
             as-is. QueryState handles loading / error / empty (binary or over
             the size cap returns null from the backend). */
          <QueryState
            isLoading={textQuery.isLoading}
            error={textQuery.error}
            isEmpty={!textQuery.isLoading && !textQuery.data}
            emptyLabel={t("library.preview.notText")}
            emptyDescription={t("library.preview.notTextDesc")}
          >
            <pre className="border-border bg-muted/50 text-foreground min-h-[60vh] w-full flex-1 overflow-auto rounded-md border p-3 font-mono text-xs leading-relaxed whitespace-pre-wrap break-words">
              {textQuery.data ? maybePrettyJson(textQuery.data) : ""}
            </pre>
          </QueryState>
        ) : (
          <iframe
            src={url}
            title={entry.name}
            // allow-same-origin so the asset URL loads; no allow-scripts so an
            // uploaded HTML file cannot run.
            sandbox="allow-same-origin"
            className="border-border bg-background min-h-[60vh] w-full flex-1 rounded-md border"
          />
        )}
      </SheetContent>
    </Sheet>
  )
}
