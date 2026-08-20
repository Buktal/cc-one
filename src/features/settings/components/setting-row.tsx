// SettingRow — the settings cards' shared row primitive: label + hint on the
// left, control on the right, hairline between rows. Content-only — rendered
// inside a section Card's CardContent, whose vertical padding comes from the
// Card (--card-spacing), so rows only carry their own inter-row spacing.
// Extracted from general-card.tsx when the 关于 card (#109) needed the same
// row shape — one implementation, two consumers.

import { Label } from "@/components/ui/label"

export function SettingRow({
  label,
  hint,
  children,
}: {
  label: string
  hint?: string
  children: React.ReactNode
}) {
  return (
    <div className="border-border flex items-start justify-between gap-4 border-t py-2.5 first:border-t-0 first:pt-0 last:pb-0">
      <div className="flex min-w-0 flex-col gap-1">
        <Label className="text-foreground">{label}</Label>
        {hint ? (
          <p className="text-muted-foreground text-xs leading-relaxed">
            {hint}
          </p>
        ) : null}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}
