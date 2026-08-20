// About card (#109) — the「关于」section that inherits the changelog entry +
// version display the old shell footer carried (removed with #105's topbar
// move). Data sources stay EXACTLY the old shell cluster's: the version reads
// useAppInfoQuery (inside UpdateControl) and the changelog entry opens GitHub
// Releases via useUpdateCheck().openReleases — the same book icon + behavior
// the footer's ChangelogVersion cluster had. UpdateIndicator rides along so a
// probed new version has a surface again (the ⓘ + auto-opening popover).
// Content-only — rendered inside SettingsView's 关于 section card.

import { BookText, ExternalLink } from "lucide-react"
import { useTranslation } from "react-i18next"
import { UpdateControl, UpdateIndicator } from "@/app/shell/update-card"
import { useUpdateCheck } from "@/app/shell/use-update-check"
import { Button } from "@/components/ui/button"
import { SettingRow } from "@/features/settings/components/setting-row"

export function AboutCard() {
  const { t } = useTranslation()
  const { openReleases } = useUpdateCheck()

  return (
    <div className="flex flex-col">
      <SettingRow
        label={t("settings.general.versionUpdate")}
        hint={t("settings.general.updateHint")}
      >
        <div className="flex items-center gap-2">
          <UpdateControl />
          <UpdateIndicator />
        </div>
      </SettingRow>
      <SettingRow
        label={t("settings.about.changelog")}
        hint={t("settings.about.changelogHint")}
      >
        <Button size="sm" variant="outline" onClick={() => void openReleases()}>
          <BookText className="size-4" />
          {t("settings.about.openChangelog")}
          <ExternalLink className="text-muted-foreground size-3" />
        </Button>
      </SettingRow>
    </div>
  )
}
