// Tooltip primitive (base-ui + render composition). Mount one
// <TooltipProvider> near the app root — a bare <Tooltip> won't position without
// it. Use <TooltipTrigger render={<Button …/>}> to wrap a custom control.

import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip"

import { cn } from "@/lib/utils"

const TooltipProvider = TooltipPrimitive.Provider

/** Tooltip root. `trackCursorAxis` lets a wide trigger (e.g. a full-column
 *  button) anchor the tooltip to the cursor instead of the trigger center —
 *  pass "both" when the trigger is much wider than the hovered text. */
function Tooltip({
  trackCursorAxis = "none",
  ...props
}: TooltipPrimitive.Root.Props) {
  return <TooltipPrimitive.Root trackCursorAxis={trackCursorAxis} {...props} />
}

function TooltipTrigger({
  className,
  ...props
}: TooltipPrimitive.Trigger.Props) {
  return (
    <TooltipPrimitive.Trigger
      data-slot="tooltip-trigger"
      className={cn(className)}
      {...props}
    />
  )
}

function TooltipContent({
  className,
  side = "top",
  align = "center",
  sideOffset = 6,
  ...props
}: TooltipPrimitive.Popup.Props &
  Pick<
    TooltipPrimitive.Positioner.Props,
    "side" | "align" | "sideOffset" | "alignOffset"
  >) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Positioner
        side={side}
        sideOffset={sideOffset}
        align={align}
        className="z-50"
      >
        <TooltipPrimitive.Popup
          data-slot="tooltip-content"
          className={cn(
            "relative max-w-xs rounded-md border border-tooltip-border bg-tooltip px-2.5 py-1 text-xs text-tooltip-foreground shadow-md data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95",
            className,
          )}
          {...props}
        />
      </TooltipPrimitive.Positioner>
    </TooltipPrimitive.Portal>
  )
}

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger }
