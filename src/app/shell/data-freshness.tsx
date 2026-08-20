// Data freshness indicator for the topbar: a live pulse + relative
// "采集于 3 分钟前". Synced mode appends "· 同步 12 分钟前". Degrades to a
// first-run hint when no collect has ever landed. The relative string is
// recomputed on each render; the topbar re-renders on its query polls, so it
// refreshes often enough.
//
// Narrow windows (≤980px) collapse the text to the bare pulse dot — the
// tooltip carries the full two-line reading, so no information is lost.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { useTranslation } from "react-i18next"

import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useFreshness } from "@/hooks/use-freshness"

// relativeTime gives us `fromNow()`; the locale it renders in is set globally by
// `setDayjsLocale` (driven from the display-language preference) — not
// hard-coded here.
dayjs.extend(relativeTime)

export function DataFreshness() {
  const { t } = useTranslation()
  const { state } = useFreshness()
  const collect = state.lastCollectAt
  const sync = state.lastSyncAt

  if (!collect) {
    return (
      <span className="text-muted-foreground text-xs">
        {t("usage.freshness.firstRun")}
      </span>
    )
  }

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span className="text-muted-foreground inline-flex items-center gap-1.5 px-1 text-[11.5px]" />
        }
      >
        <span className="relative flex size-1.5 shrink-0">
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-accent-brand opacity-75" />
          <span className="relative inline-flex size-1.5 rounded-full bg-accent-brand" />
        </span>
        <span className="max-[980px]:hidden whitespace-nowrap">
          {t("usage.freshness.collected", { ago: dayjs(collect).fromNow() })}
          {sync ? (
            <span>
              {t("usage.freshness.synced", { ago: dayjs(sync).fromNow() })}
            </span>
          ) : null}
        </span>
      </TooltipTrigger>
      <TooltipContent side="bottom">
        <div className="flex flex-col gap-0.5">
          <span>
            {t("usage.freshness.collected", { ago: dayjs(collect).fromNow() })}
          </span>
          {sync ? (
            <span>
              {t("usage.freshness.syncedPlain", { ago: dayjs(sync).fromNow() })}
            </span>
          ) : null}
        </div>
      </TooltipContent>
    </Tooltip>
  )
}
