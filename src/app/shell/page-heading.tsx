import { useTranslation } from "react-i18next"
import { useAppSelector } from "@/app/store/hooks"

/** The same page identity stays visible when the title bar collapses to icons. */
export function PageHeading() {
  const { t } = useTranslation()
  const view = useAppSelector((s) => s.view.view)

  return (
    <div className="page-heading">
      <h1 className="font-heading text-xl font-semibold tracking-tight">
        {t(`nav.${view}`)}
      </h1>
      <p className="text-muted-foreground text-xs leading-relaxed">
        {t(`pageDescription.${view}`)}
      </p>
    </div>
  )
}
