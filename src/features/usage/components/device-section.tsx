// Device section (#107) — the dashboard's device dimension at usage grain:
// one row per device with usage in the window (GROUP BY omits empty buckets —
// a silent peer is invisible by design, and the section head's summary counts
// the silent ones out). Row shape follows the section system's DistRow:
// device name (+「本机」mark on this machine) · `数量 · 占比` · bar · sub line
// 请求 · 命中率 · 最近活跃/同步. Naming / self identity join from the device
// REGISTRY (`listDevices`, the dropdown's source) — the endpoint carries pure
// usage aggregates, the same division as the device filter. Clicking a row
// narrows the shared device_scope filter (the project ranking's interaction);
// single-device setups render no pick — nothing to switch to, the same caliber
// as DeviceScopeControl.

import dayjs from "dayjs"
import { useTranslation } from "react-i18next"
import { useDevicesQuery, useDeviceUsageQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { type FilterState, patchFilter } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { deviceSectionStats } from "@/features/usage/derive"
import {
  formatCount,
  formatMetricLine,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTokens,
} from "@/lib/format"
import { deviceOptionLabel } from "../use-device-options"
import { DistRow } from "./dist-row"

export function DeviceSection({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const selected = useAppSelector((s) => s.filter.filter.device_scope)
  const { data: rows = [], isLoading, error } = useDeviceUsageQuery(filter)
  const { data: devices = [] } = useDevicesQuery()
  const stats = deviceSectionStats(rows)
  // Registry join: label + is_self from listDevices, so the ONE label
  // derivation (deviceOptionLabel) keeps feeding every device surface. A
  // usage device missing from the registry (self-heal gap) falls back to its
  // raw id — labeled, never a crash.
  const meta = new Map(
    devices.map((d) => [
      d.device_id,
      { label: deviceOptionLabel(d, t), is_self: d.is_self },
    ]),
  )
  // 单设备（Standalone 仅本机）没有切换目标 — 行不可点（DeviceScopeControl
  // 同口径不渲染）。多设备时点行写入共享 device_scope 筛选，再点取消。
  const clickable = devices.length > 1
  const pick = (id: string) =>
    dispatch(patchFilter({ device_scope: selected === id ? "" : id }))

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={rows.length === 0}
      emptyLabel={t("usage.devices.empty")}
      emptyDescription={t("usage.devices.emptyDesc")}
    >
      <Card interactive>
        <CardHeader>
          <CardTitle>{t("usage.devices.rankTitle")}</CardTitle>
          <span className="text-muted-foreground/70 self-end text-xs">
            {t("usage.devices.count", { n: formatCount(stats.devices) })}
          </span>
        </CardHeader>
        <CardContent className="flex flex-col gap-1.5">
          {rows.map((r) => {
            const m = meta.get(r.device_id)
            return (
              <DistRow
                key={r.device_id}
                mono={!m}
                name={m?.label ?? r.device_id}
                badge={m?.is_self ? t("devices.thisDevice") : undefined}
                value={formatSegValue(
                  formatTokens(r.total_tokens),
                  r.total_tokens / stats.totalTokens,
                )}
                share={r.total_tokens / stats.totalTokens}
                sub={formatMetricLine([
                  formatMetricSeg(
                    t("usage.hero.requests"),
                    formatCount(r.request_count),
                  ),
                  formatMetricSeg(
                    t("usage.hero.cacheHitRate"),
                    formatPct(r.cache_hit_rate),
                  ),
                  // 本机的 recency 是「最近活跃」（实时采集）；对端的数据随
                  // pull 到达，展示为「最近同步」。
                  formatMetricSeg(
                    m?.is_self
                      ? t("usage.devices.lastActive")
                      : t("usage.devices.lastSync"),
                    dayjs(r.last_active_at).fromNow(),
                  ),
                ])}
                selected={selected === r.device_id}
                onClick={clickable ? () => pick(r.device_id) : undefined}
              />
            )
          })}
        </CardContent>
        <CardFooter className="text-muted-foreground/70 gap-2 text-[10.5px]">
          <span>{t("usage.devices.shareNote")}</span>
        </CardFooter>
      </Card>
    </QueryState>
  )
}
