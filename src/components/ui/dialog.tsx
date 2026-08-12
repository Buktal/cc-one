// Dialog primitive (base-ui + render composition). Composes Portal
// + Backdrop + Popup into a single <DialogContent>. base-ui's Dialog has no
// Positioner (unlike Select/Tooltip), so the popup is centered with
// fixed + translate. onOpenChange keeps base-ui's (open, details) signature;
// callers can pass (open) => setOpen(open) and ignore details.

import { Dialog as DialogPrimitive } from "@base-ui/react/dialog"
import { XIcon } from "lucide-react"
import type * as React from "react"

import { cn } from "@/lib/utils"

const Dialog = DialogPrimitive.Root
const DialogTrigger = DialogPrimitive.Trigger
const DialogPortal = DialogPrimitive.Portal
const DialogClose = DialogPrimitive.Close

function DialogBackdrop({
  className,
  ...props
}: DialogPrimitive.Backdrop.Props) {
  return (
    <DialogPrimitive.Backdrop
      data-slot="dialog-backdrop"
      className={cn(
        "fixed inset-0 z-50 bg-black/60 data-closed:animate-out data-closed:fade-out-0 data-open:animate-in data-open:fade-in-0",
        className,
      )}
      {...props}
    />
  )
}

function DialogContent({
  className,
  children,
  showClose = true,
  ...props
}: DialogPrimitive.Popup.Props & {
  /** Render the top-right close button. @default true */
  showClose?: boolean
}) {
  return (
    <DialogPortal>
      <DialogBackdrop />
      <DialogPrimitive.Popup
        data-slot="dialog-content"
        className={cn(
          "fixed top-1/2 left-1/2 z-50 grid w-full max-w-lg -translate-x-1/2 -translate-y-1/2 gap-4 rounded-lg border bg-popover p-6 text-popover-foreground shadow-lg ring-1 ring-foreground/5 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 dark:ring-foreground/10",
          className,
        )}
        {...props}
      >
        {showClose ? (
          <DialogPrimitive.Close
            render={
              <button
                type="button"
                aria-label="Close"
                className="absolute top-3 right-3 rounded-md p-1 text-muted-foreground opacity-70 transition-opacity duration-150 hover:bg-muted hover:opacity-100 focus-visible:ring-2 focus-visible:ring-ring/40 focus-visible:outline-none [&_svg]:pointer-events-none [&_svg]:size-4"
              />
            }
          >
            <XIcon />
          </DialogPrimitive.Close>
        ) : null}
        {children}
      </DialogPrimitive.Popup>
    </DialogPortal>
  )
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-header"
      className={cn("flex flex-col gap-1.5 text-left", className)}
      {...props}
    />
  )
}

function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn(
        "flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
        className,
      )}
      {...props}
    />
  )
}

function DialogTitle({ className, ...props }: DialogPrimitive.Title.Props) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn("font-heading text-base font-semibold", className)}
      {...props}
    />
  )
}

function DialogDescription({
  className,
  ...props
}: DialogPrimitive.Description.Props) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  )
}

export {
  Dialog,
  DialogBackdrop,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
}
