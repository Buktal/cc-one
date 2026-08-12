// Sheet primitive (base-ui Drawer + render composition). base-ui
// exposes Drawer as a mobile-style bottom sheet; we repurpose it as a desktop
// side panel via the `side` prop on <SheetContent>. Manual Portal + Backdrop +
// Popup composition (we don't use Drawer.Content so we fully control layout).

import { Drawer as DrawerPrimitive } from "@base-ui/react/drawer"
import { XIcon } from "lucide-react"
import type * as React from "react"

import { cn } from "@/lib/utils"

const Sheet = DrawerPrimitive.Root
const SheetTrigger = DrawerPrimitive.Trigger
const SheetClose = DrawerPrimitive.Close

const SIDE_CLASSES: Record<NonNullable<SheetContentProps["side"]>, string> = {
  right:
    "inset-y-0 right-0 h-full w-full max-w-sm border-l data-closed:slide-out-to-right data-open:slide-in-from-right",
  left: "inset-y-0 left-0 h-full w-full max-w-sm border-r data-closed:slide-out-to-left data-open:slide-in-from-left",
  top: "inset-x-0 top-0 max-h-[85vh] w-full border-b data-closed:slide-out-to-top data-open:slide-in-from-top",
  bottom:
    "inset-x-0 bottom-0 max-h-[85vh] w-full border-t data-closed:slide-out-to-bottom data-open:slide-in-from-bottom",
}

type SheetContentProps = DrawerPrimitive.Popup.Props & {
  side?: "top" | "right" | "bottom" | "left"
  /** Render the top-right close button. @default true */
  showClose?: boolean
}

function SheetContent({
  className,
  children,
  side = "right",
  showClose = true,
  ...props
}: SheetContentProps) {
  return (
    <DrawerPrimitive.Portal>
      <DrawerPrimitive.Backdrop
        data-slot="sheet-backdrop"
        className="fixed inset-0 z-50 bg-black/60 data-closed:animate-out data-closed:fade-out-0 data-open:animate-in data-open:fade-in-0"
      />
      <DrawerPrimitive.Popup
        data-slot="sheet-content"
        data-side={side}
        className={cn(
          "fixed z-50 flex flex-col gap-4 bg-popover p-6 text-popover-foreground shadow-lg data-closed:animate-out data-open:animate-in",
          SIDE_CLASSES[side],
          className,
        )}
        {...props}
      >
        {showClose ? (
          <DrawerPrimitive.Close
            render={
              <button
                type="button"
                aria-label="Close"
                className="absolute top-4 right-4 rounded-md p-1 text-muted-foreground opacity-70 transition-opacity duration-150 hover:bg-muted hover:opacity-100 focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none [&_svg]:pointer-events-none [&_svg]:size-4"
              />
            }
          >
            <XIcon />
          </DrawerPrimitive.Close>
        ) : null}
        {children}
      </DrawerPrimitive.Popup>
    </DrawerPrimitive.Portal>
  )
}

function SheetHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-header"
      className={cn("flex flex-col gap-1.5", className)}
      {...props}
    />
  )
}

function SheetFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="sheet-footer"
      className={cn(
        "mt-auto flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
        className,
      )}
      {...props}
    />
  )
}

function SheetTitle({ className, ...props }: DrawerPrimitive.Title.Props) {
  return (
    <DrawerPrimitive.Title
      data-slot="sheet-title"
      className={cn("font-heading text-base font-semibold", className)}
      {...props}
    />
  )
}

function SheetDescription({
  className,
  ...props
}: DrawerPrimitive.Description.Props) {
  return (
    <DrawerPrimitive.Description
      data-slot="sheet-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  )
}

export {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
}
