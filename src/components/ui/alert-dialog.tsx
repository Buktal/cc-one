// AlertDialog primitive (base-ui + render composition) — the destructive-
// action confirmation. base-ui's AlertDialog always disables pointer
// dismissal (backdrop click never closes it), while ESC does close by
// default; <AlertDialogContent onBackdropClick> opts back in to backdrop
// clicks when a caller wants "click outside = cancel". Composes Portal +
// Backdrop + Popup into a single <AlertDialogContent>, mirroring dialog.tsx;
// the action button is a plain Button (it does not auto-close — callers
// close + run the mutation in its onClick).

import { AlertDialog as AlertDialogPrimitive } from "@base-ui/react/alert-dialog"
import type * as React from "react"

import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

const AlertDialog = AlertDialogPrimitive.Root
const AlertDialogTrigger = AlertDialogPrimitive.Trigger
const AlertDialogPortal = AlertDialogPrimitive.Portal

function AlertDialogOverlay({
  className,
  ...props
}: AlertDialogPrimitive.Backdrop.Props) {
  return (
    <AlertDialogPrimitive.Backdrop
      data-slot="alert-dialog-overlay"
      className={cn(
        "fixed inset-0 z-50 bg-black/60 data-closed:animate-out data-closed:fade-out-0 data-open:animate-in data-open:fade-in-0",
        className,
      )}
      {...props}
    />
  )
}

function AlertDialogContent({
  className,
  children,
  onBackdropClick,
  ...props
}: AlertDialogPrimitive.Popup.Props & {
  /** Backdrop-click callback — base-ui's AlertDialog never dismisses on an
   *  outside press, so callers opt in explicitly ("click outside = cancel"). */
  onBackdropClick?: () => void
}) {
  return (
    <AlertDialogPortal>
      <AlertDialogOverlay onClick={onBackdropClick} />
      <AlertDialogPrimitive.Popup
        data-slot="alert-dialog-content"
        className={cn(
          "fixed top-1/2 left-1/2 z-50 grid w-full max-w-sm -translate-x-1/2 -translate-y-1/2 gap-4 rounded-lg border bg-popover p-6 text-popover-foreground shadow-lg ring-1 ring-foreground/5 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 dark:ring-foreground/10",
          className,
        )}
        {...props}
      >
        {children}
      </AlertDialogPrimitive.Popup>
    </AlertDialogPortal>
  )
}

function AlertDialogHeader({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-dialog-header"
      className={cn("flex flex-col gap-1.5 text-left", className)}
      {...props}
    />
  )
}

function AlertDialogFooter({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="alert-dialog-footer"
      className={cn(
        // gap-3 (12px): the two actions sit close enough to read as one group,
        // with room to breathe between cancel and the destructive confirm.
        "flex flex-col-reverse gap-3 sm:flex-row sm:justify-end",
        className,
      )}
      {...props}
    />
  )
}

function AlertDialogTitle({
  className,
  ...props
}: AlertDialogPrimitive.Title.Props) {
  return (
    <AlertDialogPrimitive.Title
      data-slot="alert-dialog-title"
      className={cn("font-heading text-base font-semibold", className)}
      {...props}
    />
  )
}

function AlertDialogDescription({
  className,
  ...props
}: AlertDialogPrimitive.Description.Props) {
  return (
    <AlertDialogPrimitive.Description
      data-slot="alert-dialog-description"
      className={cn("text-sm text-muted-foreground", className)}
      {...props}
    />
  )
}

// base-ui's Close is a bare unstyled <button> — render it as an outline Button
// so Cancel reads as a real button, weight-matched against the destructive
// Action (a bare text link next to a filled red button looks broken).
// Function-form render spreads every base-ui prop explicitly onto the Button,
// so the outline styles can never be dropped by prop merging.
function AlertDialogCancel({
  className,
  ...props
}: AlertDialogPrimitive.Close.Props) {
  return (
    <AlertDialogPrimitive.Close
      render={(renderProps) => (
        <Button
          variant="outline"
          className={cn(className, renderProps.className)}
          {...renderProps}
        />
      )}
      {...props}
    />
  )
}

function AlertDialogAction({
  className,
  ...props
}: React.ComponentProps<typeof Button>) {
  return (
    <Button
      data-slot="alert-dialog-action"
      className={cn(className)}
      {...props}
    />
  )
}

export {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogOverlay,
  AlertDialogPortal,
  AlertDialogTitle,
  AlertDialogTrigger,
}
