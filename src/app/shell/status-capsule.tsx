// 顶栏状态胶囊：设备身份 + 数据新鲜度的单一入口。收拢了原左簇身份区
// （模式徽标 + 设备名 tooltip）与原右簇 DataFreshness——两个系统状态原先
// 分居两端、且随窄窗逐级消失（≤840 身份簇整个隐藏后同步状态彻底不可见）；
// 胶囊恒在场，最窄退化为一个心跳点，点开 Popover 仍是全部信息。
//
// 断点退化（纯 CSS）：≤1360 藏「· N 前」时间 / ≤980 藏设备名；心跳点恒显。
// Popover 与胶囊的分工：胶囊常驻给相对新鲜度（多新鲜），Popover 详情给
// 精确时刻（何时）——身份区（设备名为题、模式 + 设备号并为元信息一行，
// 设备名是号的截断默认命名，分开两行近乎重复）hairline 分组后是采集/同步
// 的 label-时刻两列（tabular-nums 对齐）。首启（从未采集过）给引导文案。
// 相对时间随顶栏查询轮询重算（与原 DataFreshness 同一策略）。

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { useTranslation } from "react-i18next"

import { useAppInfoQuery } from "@/app/store/api"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { useFreshness } from "@/hooks/use-freshness"
import { formatTime } from "@/lib/format"
import { cn } from "@/lib/utils"

// relativeTime gives us `fromNow()`; the locale it renders in is set globally by
// `setDayjsLocale` (driven from the display-language preference) — not
// hard-coded here.
dayjs.extend(relativeTime)

/** 心跳点：有数据 = 品牌 ping（顶栏唯一持续动画元素，数据的「生命体征」）；
 *  首启 = 静态灰点（还没有可跳的心跳）。 */
function Pulse({ live }: { live: boolean }) {
  if (!live) {
    return (
      <span className="bg-muted-foreground/60 size-1.5 shrink-0 rounded-full" />
    )
  }
  return (
    <span className="relative flex size-1.5 shrink-0">
      <span className="absolute inline-flex size-full animate-ping rounded-full bg-accent-brand opacity-75" />
      <span className="relative inline-flex size-1.5 rounded-full bg-accent-brand" />
    </span>
  )
}

export function StatusCapsule() {
  const { t } = useTranslation()
  const { data: info } = useAppInfoQuery()
  const { state } = useFreshness()
  const collect = state.lastCollectAt
  const sync = state.lastSyncAt
  const synced = info?.mode === "synced"
  const deviceName = info?.display_name || t("common.unnamed")

  return (
    <Popover>
      <PopoverTrigger
        render={
          <button
            type="button"
            aria-label={t("shell.status.aria")}
            className="text-muted-foreground hover:bg-hover hover:text-foreground inline-flex h-7 max-w-56 items-center gap-1.5 rounded-full px-2.5 text-[11.5px] transition-colors"
          />
        }
      >
        <Pulse live={collect !== null} />
        {collect ? (
          <>
            <span className="max-[980px]:hidden min-w-0 truncate">
              {deviceName}
            </span>
            <span className="max-[1360px]:hidden whitespace-nowrap">
              · {dayjs(collect).fromNow()}
            </span>
          </>
        ) : (
          <span className="max-[980px]:hidden whitespace-nowrap">
            {t("shell.status.none")}
          </span>
        )}
      </PopoverTrigger>
      <PopoverContent align="end" className="w-60 p-3.5">
        {/* 身份：设备名为题；模式 + 设备号并为元信息一行。已同步是关键
            状态用品牌色字，不再用 chip——Popover 是详情层，文字更轻。 */}
        <div className="text-sm font-medium">{deviceName}</div>
        <div className="text-muted-foreground mt-0.5 flex items-baseline gap-1.5 text-xs">
          <span className={cn(synced && "text-accent-brand-strong")}>
            {t(synced ? "shell.synced" : "shell.standalone")}
          </span>
          <span>·</span>
          <span className="truncate">{info?.device_id ?? "—"}</span>
        </div>
        {/* 数据：hairline 与身份分组；label 左 / 时刻右，tabular-nums 对齐。 */}
        {collect ? (
          <dl className="mt-3 border-t border-border pt-2.5 text-xs">
            <div className="flex items-baseline justify-between gap-4">
              <dt className="text-muted-foreground">
                {t("usage.freshness.collected")}
              </dt>
              <dd className="tabular-nums">{formatTime(collect)}</dd>
            </div>
            {sync ? (
              <div className="mt-1 flex items-baseline justify-between gap-4">
                <dt className="text-muted-foreground">
                  {t("usage.freshness.syncedPlain")}
                </dt>
                <dd className="tabular-nums">{formatTime(sync)}</dd>
              </div>
            ) : null}
          </dl>
        ) : (
          <p className="text-muted-foreground mt-3 border-t border-border pt-2.5 text-xs">
            {t("usage.freshness.firstRun")}
          </p>
        )}
      </PopoverContent>
    </Popover>
  )
}
