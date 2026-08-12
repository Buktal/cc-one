// Shared one-click copy button: Copy → Check icon flip with an aria label.
// Success feedback stays in the icon (copying is instant and self-evident —
// a toast on every click would be noise); failure (clipboard denied) toasts
// once so the user isn't left wondering whether it worked.

import { Check, Copy } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { cn } from "@/lib/utils"

export function CopyButton({
  value,
  label,
  className,
}: {
  value: string
  /** Accessible name for the icon button, e.g. "Copy device ID". */
  label: string
  className?: string
}) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)

  async function copy() {
    try {
      await navigator.clipboard.writeText(value)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      toast.error(t("common.copyFailed"))
    }
  }

  return (
    <Button
      variant="ghost"
      size="icon-sm"
      className={cn("size-5 shrink-0 text-muted-foreground", className)}
      aria-label={label}
      onClick={copy}
    >
      {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
    </Button>
  )
}
