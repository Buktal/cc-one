// Data freshness indicator: a live pulse + relative "采集于 3 分钟前". Synced
// mode appends "· 同步 12 分钟前". Degrades to a first-run hint when no collect
// has ever landed. The relative string is recomputed on each render; callers
// that want it to tick over time can re-render on a timer (the views hosting
// it re-render on their query polls, so it refreshes often enough).
//
// Two layouts: `stacked` renders each time on its own line (the sidebar — one
// line with both times overflows the 12rem column); the default single line
// keeps the old log-toolbar look.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { useTranslation } from "react-i18next"

import { useFreshness } from "@/hooks/use-freshness"

// relativeTime gives us `fromNow()`; the locale it renders in is set globally by
// `setDayjsLocale` (driven from the display-language preference) — not
// hard-coded here.
dayjs.extend(relativeTime)

export function DataFreshness({ stacked = false }: { stacked?: boolean }) {
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

  if (stacked) {
    return (
      <div className="text-muted-foreground flex flex-col gap-1 text-[11px] leading-snug">
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="relative flex size-1.5 shrink-0">
            <span className="absolute inline-flex size-full animate-ping rounded-full bg-accent-brand opacity-75" />
            <span className="relative inline-flex size-1.5 rounded-full bg-accent-brand" />
          </span>
          <span className="truncate">
            {t("usage.freshness.collected", { ago: dayjs(collect).fromNow() })}
          </span>
        </span>
        {sync ? (
          // syncedPlain: no " · " prefix — the single-line variant uses it as a
          // joiner, but on its own stacked line it reads as noise. pl-3 aligns
          // this line's text with the collected line's (6px dot + 6px gap).
          <span className="truncate pl-3">
            {t("usage.freshness.syncedPlain", { ago: dayjs(sync).fromNow() })}
          </span>
        ) : null}
      </div>
    )
  }

  return (
    <div className="flex items-center gap-2 text-xs">
      <span className="relative flex size-1.5">
        <span className="absolute inline-flex size-full animate-ping rounded-full bg-accent-brand opacity-75" />
        <span className="relative inline-flex size-1.5 rounded-full bg-accent-brand" />
      </span>
      <span className="text-muted-foreground">
        <span>
          {t("usage.freshness.collected", { ago: dayjs(collect).fromNow() })}
        </span>
        {sync ? (
          <span>
            {t("usage.freshness.synced", { ago: dayjs(sync).fromNow() })}
          </span>
        ) : null}
      </span>
    </div>
  )
}
