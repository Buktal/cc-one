// Request log table — per-API-call ledger. Columns: Time / Source / Billed
// Model / 输入 / 输出 / 缓存创建 / 缓存命中 / 总 Token / Cost / 停止原因 /
// Device. The Source cell shows the human-readable tag (`sourceLabel`), with the
// raw tag (e.g. `claude_code`) in the title tooltip. `stop_reason` (end_turn /
// tool_use / max_tokens …) is the
// per-call end semantic. No latency / TTFT / HTTP-status columns.
// Fixed time-desc (no sort UI); paginated; empty state offers an inline 采集
// CTA so the user isn't bounced to the command bar to seed the first rows.

import { FileText } from "lucide-react"
import { type ReactNode, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"

import { useCountQuery, useLogsQuery } from "@/app/store/api"
import { QueryState } from "@/components/query-state"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { stopReasonTone } from "@/features/usage/derive"
import { useCollectAction } from "@/hooks/use-collect-action"
import { formatCost, formatInt, formatTime } from "@/lib/format"
import { paginate } from "@/lib/pagination"
import { tokenTotal } from "@/lib/usage"
import { cn } from "@/lib/utils"
import type { UsageFilter } from "@/types/generated/bindings"
import { sourceLabel } from "../source-labels"
import { useDeviceLabelMap, useDeviceOptions } from "../use-device-options"

const PAGE_SIZE = 20

/**
 * Right-aligned token-column header: label + a muted, language-neutral `tok`
 * unit. Cells stay pure numbers (tabular-nums) — the unit rides the header so
 * the dense ledger stays scannable (consistent with the recent-requests card).
 */
function TokHead({ children }: { children: ReactNode }) {
  return (
    <TableHead className="text-right">
      <span className="inline-flex items-center justify-end gap-1">
        {children}
        <span className="text-muted-foreground text-[10px] font-normal">
          tok
        </span>
      </span>
    </TableHead>
  )
}

export function RequestLogTable({ filter }: { filter: UsageFilter }) {
  const { t } = useTranslation()
  const deviceLabel = useDeviceLabelMap()
  const [offset, setOffset] = useState(0)
  // Reset to page 1 when the filter changes — otherwise a narrower filter
  // (e.g. fewer rows after switching model/device) can land on an empty page.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — offset resets on filter change; the body needs no filter value
  useEffect(() => setOffset(0), [filter])
  const {
    data: rows = [],
    isLoading,
    error,
  } = useLogsQuery({
    filter,
    limit: PAGE_SIZE,
    offset,
  })
  const { data: total = 0 } = useCountQuery(filter)
  // 空状态 CTA 复用 sidebar 同一份采集动作 (useCollectAction) —— 不再在此
  // 手写 mutation + toast, 避免分叉 (上一份手写副本就漏了数据新鲜度戳记
  // markCollected/markSynced). multiDevice 决定成功 toast 措辞, 与 shell 一致.
  const multiDevice = useDeviceOptions().length > 0
  const { onCollect, collecting } = useCollectAction(multiDevice)

  const { totalPages, page } = paginate(total, offset, PAGE_SIZE)

  return (
    <Card className="min-h-0 flex-1">
      <CardHeader>
        <CardTitle>{t("usage.logs.title")}</CardTitle>
      </CardHeader>
      <CardContent className="flex min-h-0 flex-1 flex-col">
        <QueryState
          isLoading={isLoading}
          error={error}
          isEmpty={!isLoading && rows.length === 0}
          emptyIcon={FileText}
          emptyLabel={t("usage.logs.empty")}
          emptyAction={{
            label: collecting
              ? t("usage.collect.collecting")
              : t("usage.collect.collectLocal"),
            onClick: onCollect,
            disabled: collecting,
          }}
        >
          <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>{t("usage.logs.col.time")}</TableHead>
                  <TableHead>{t("usage.logs.col.source")}</TableHead>
                  <TableHead>{t("usage.logs.col.billedModel")}</TableHead>
                  <TokHead>{t("usage.tokens.input")}</TokHead>
                  <TokHead>{t("usage.tokens.output")}</TokHead>
                  <TokHead>{t("usage.tokens.cacheCreation")}</TokHead>
                  <TokHead>{t("usage.tokens.cacheRead")}</TokHead>
                  <TokHead>{t("usage.logs.col.totalToken")}</TokHead>
                  <TableHead className="text-right">
                    {t("usage.logs.col.cost")}
                  </TableHead>
                  <TableHead>{t("usage.logs.col.stopReason")}</TableHead>
                  <TableHead>{t("usage.logs.col.device")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((r) => (
                  <TableRow key={r.uuid}>
                    <TableCell className="tabular-nums whitespace-nowrap">
                      {formatTime(r.timestamp)}
                    </TableCell>
                    <TableCell title={r.source}>
                      {sourceLabel(r.source) || "—"}
                    </TableCell>
                    <TableCell className="font-mono text-xs">
                      {r.model}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatInt(r.tokens.input)}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatInt(r.tokens.output)}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatInt(r.tokens.cache_creation)}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatInt(r.tokens.cache_read)}
                    </TableCell>
                    <TableCell className="text-right font-medium tabular-nums">
                      {formatInt(tokenTotal(r))}
                    </TableCell>
                    <TableCell className="text-right tabular-nums">
                      {formatCost(r.total_cost_usd)}
                    </TableCell>
                    <TableCell>
                      <StopReasonCell value={r.stop_reason} />
                    </TableCell>
                    <TableCell
                      className="text-muted-foreground text-xs"
                      title={r.device_id || undefined}
                    >
                      {r.device_id
                        ? (deviceLabel.get(r.device_id) ??
                          r.device_id.slice(0, 8))
                        : "—"}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </QueryState>

        <div className="text-muted-foreground mt-3 flex shrink-0 items-center justify-between text-xs">
          <span>
            {t("usage.logs.pageInfo", {
              page,
              totalPages,
              total: formatInt(total),
            })}
          </span>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={offset === 0}
              onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
            >
              {t("usage.logs.prevPage")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={offset + PAGE_SIZE >= total}
              onClick={() => setOffset(offset + PAGE_SIZE)}
            >
              {t("usage.logs.nextPage")}
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>
  )
}

function StopReasonCell({ value }: { value: string }) {
  if (!value) return <span className="text-muted-foreground">—</span>
  const tone = stopReasonTone(value)
  if (!tone)
    return (
      <span className="text-muted-foreground font-mono text-xs">{value}</span>
    )
  return <span className={cn("sr-chip font-mono", `sr-${tone}`)}>{value}</span>
}
