// Theme toggle — cycles light → dark → system (next-themes). Icon-only with a
// tooltip; mounted in the shell header (landscape) and sidebar footer.

import { Monitor, Moon, Sun } from "lucide-react"
import { useTheme } from "next-themes"
import { useTranslation } from "react-i18next"

import { Button } from "@/components/ui/button"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"

const NEXT: Record<string, "light" | "dark" | "system"> = {
  light: "dark",
  dark: "system",
  system: "light",
}

const META: Record<string, { Icon: typeof Sun; key: string }> = {
  light: { Icon: Sun, key: "theme.light" },
  dark: { Icon: Moon, key: "theme.dark" },
  system: { Icon: Monitor, key: "theme.system" },
}

export function ThemeToggle() {
  const { t } = useTranslation()
  const { theme = "system", setTheme } = useTheme()
  const { Icon, key } = META[theme] ?? META.system
  const label = t(key)

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon-sm"
            className="text-muted-foreground"
            aria-label={t("theme.aria", { current: label })}
            onClick={() => setTheme(NEXT[theme] ?? "dark")}
          />
        }
      >
        <Icon />
      </TooltipTrigger>
      <TooltipContent>{t("theme.tooltip", { current: label })}</TooltipContent>
    </Tooltip>
  )
}
