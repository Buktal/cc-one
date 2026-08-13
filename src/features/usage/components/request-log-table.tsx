// Request log table — per-API-call ledger. Columns: Time / App / Billed
// Model / Total / Cost / Stop / Device. The four token buckets and the cost
// breakdown live in the expandable row detail (one row click) instead of five
// extra columns, so the ledger stays scannable; the Source cell shows the
// human-readable tag (`sourceLabel`) with the raw tag in the title tooltip.
// stop_reason renders a localized chip (`stopReasonLabelKey`) with the raw
// English value in the tooltip; calls costing ≥ COST_NOTABLE_THRESHOLD get a
// highlighted cost cell. Rows are time-desc grouped by local day with a day
// separator. Paginated with the shared PaginationBar (ellipsis jumps); the
// bar disables while a page refetches. Empty state offers an inline 采集 CTA
// so the user isn't bounced to the command bar to seed the first rows.

import { FileText } from "lucide-react"
import { Fragment, type ReactNode, useEffect, useState } from "react"
import { useTranslation } from "react-i18next"

import { useCountQuery, useLogsQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { CopyButton } from "@/components/copy-button"
import { PaginationBar } from "@/components/pagination-bar"
import { QueryState } from "@/components/query-state"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  classifyStopReason,
  costIsNotable,
  groupRowsByDay,
} from "@/features/usage/derive"
import { useCollectAction } from "@/hooks/use-collect-action"
import {
  formatCost,
  formatDay,
  formatInt,
  formatTime,
  formatTokens,
} from "@/lib/format"
import { paginate } from "@/lib/pagination"
import { tokenTotal } from "@/lib/usage"
import { cn } from "@/lib/utils"
import type { UsageLogRow } from "@/types/generated/bindings"
import { sourceLabel } from "../source-labels"
import { useDeviceLabelMap, useDeviceOptions } from "../use-device-options"

const PAGE_SIZE = 20

export function RequestLogTable({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const deviceLabel = useDeviceLabelMap()
  const [offset, setOffset] = useState(0)
  const [expandedId, setExpandedId] = useState<string | null>(null)
  // Reset to page 1 and collapse any expanded row when the filter changes —
  // a narrower filter (e.g. fewer rows after switching model/device) can land
  // on an empty page, and the expanded row's page may no longer exist.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — offset/expanded reset on filter change; the body needs no filter value
  useEffect(() => {
    setOffset(0)
    setExpandedId(null)
  }, [filter])
  const {
    data: rows = [],
    isLoading,
    isFetching,
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
  const dayGroups = groupRowsByDay(rows)

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
                  <TableHead className="text-right">
                    {t("usage.logs.col.totalToken")}
                  </TableHead>
                  <TableHead className="text-right">
                    {t("usage.logs.col.cost")}
                  </TableHead>
                  <TableHead>{t("usage.logs.col.stopReason")}</TableHead>
                  <TableHead>{t("usage.logs.col.device")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {dayGroups.map((g) => (
                  <Fragment key={g.dayKey}>
                    {/* Day separator — the ledger stays grouped by local day
                      across multi-day windows. */}
                    <TableRow className="bg-muted/40 border-border/60">
                      <TableCell
                        colSpan={7}
                        className="text-muted-foreground px-3 py-1 text-[11px] font-medium"
                      >
                        {formatDay(g.dayKey)}
                      </TableCell>
                    </TableRow>
                    {g.rows.map((r) => (
                      <Fragment key={r.uuid}>
                        <LogRow
                          r={r}
                          deviceLabel={deviceLabel}
                          expanded={expandedId === r.uuid}
                          onToggle={() =>
                            setExpandedId(expandedId === r.uuid ? null : r.uuid)
                          }
                        />
                        {expandedId === r.uuid ? <DetailRow r={r} /> : null}
                      </Fragment>
                    ))}
                  </Fragment>
                ))}
              </TableBody>
            </Table>
          </div>
        </QueryState>

        {total > 0 ? (
          <PaginationBar
            page={page}
            totalPages={totalPages}
            total={total}
            loading={isFetching}
            onPageChange={(p) => setOffset((p - 1) * PAGE_SIZE)}
          />
        ) : null}
      </CardContent>
    </Card>
  )
}

function LogRow({
  r,
  deviceLabel,
  expanded,
  onToggle,
}: {
  r: UsageLogRow
  deviceLabel: Map<string, string>
  expanded: boolean
  onToggle: () => void
}) {
  const notable = costIsNotable(r.total_cost_usd)
  return (
    <TableRow
      className={cn("cursor-pointer", expanded && "bg-hover")}
      onClick={onToggle}
      aria-expanded={expanded}
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault()
          onToggle()
        }
      }}
    >
      <TableCell className="tabular-nums whitespace-nowrap">
        {formatTime(r.timestamp)}
      </TableCell>
      <TableCell>
        <Tooltip>
          <TooltipTrigger
            render={<span>{sourceLabel(r.source) || "—"}</span>}
          />
          <TooltipContent>{r.source}</TooltipContent>
        </Tooltip>
      </TableCell>
      <TableCell className="font-mono text-xs">
        <Tooltip>
          <TooltipTrigger
            render={
              <span className="inline-block max-w-48 truncate align-bottom">
                {r.model}
              </span>
            }
          />
          <TooltipContent>{r.model}</TooltipContent>
        </Tooltip>
      </TableCell>
      <TableCell className="text-right tabular-nums">
        <Tooltip>
          <TooltipTrigger render={<span>{formatTokens(tokenTotal(r))}</span>} />
          <TooltipContent>{formatInt(tokenTotal(r))}</TooltipContent>
        </Tooltip>
      </TableCell>
      <TableCell
        className={cn(
          "text-right tabular-nums",
          notable && "text-[var(--sr-warn)] font-semibold",
        )}
      >
        {formatCost(r.total_cost_usd)}
      </TableCell>
      <TableCell>
        <StopReasonCell value={r.stop_reason} />
      </TableCell>
      <TableCell className="text-muted-foreground text-xs">
        {r.device_id ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <span>
                  {deviceLabel.get(r.device_id) ?? r.device_id.slice(0, 8)}
                </span>
              }
            />
            <TooltipContent>{r.device_id}</TooltipContent>
          </Tooltip>
        ) : (
          "—"
        )}
      </TableCell>
    </TableRow>
  )
}

function StopReasonCell({ value }: { value: string }) {
  const { t } = useTranslation()
  if (!value) return <span className="text-muted-foreground">—</span>
  const { tone, labelKey } = classifyStopReason(value)
  if (!tone || !labelKey)
    return (
      <span className="text-muted-foreground font-mono text-xs">{value}</span>
    )
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span className={cn("sem-chip font-mono", `sr-${tone}`)} />}
      >
        {t(labelKey)}
      </TooltipTrigger>
      <TooltipContent>{value}</TooltipContent>
    </Tooltip>
  )
}

/** Cost breakdown section line: label + token count + amount, reused by the
 *  detail row. `tokens` renders as a muted prefix ("1.2k →") so the eye reads
 *  token → 金额 at a glance; a line without token data (e.g. totals) shows the
 *  amount alone. */
function CostLine({
  label,
  tokens,
  value,
}: {
  label: ReactNode
  tokens?: string
  value: string
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <span className="text-muted-foreground text-xs">{label}</span>
      <span className="tabular-nums text-xs">
        {tokens ? (
          <span className="text-muted-foreground mr-1.5">{tokens} →</span>
        ) : null}
        {value}
      </span>
    </div>
  )
}

/** Expandable per-row detail — cost buckets, session, tier, iterations, tool
 *  use, request ID (copyable). All fields come from the row payload itself
 *  (the log query selects the full record), so expanding costs no round-trip. */
function DetailRow({ r }: { r: UsageLogRow }) {
  const { t } = useTranslation()

  const tools: ReactNode[] = []
  if (r.server_tool_use.web_search > 0)
    tools.push(
      t("usage.logs.detail.webSearch", { n: r.server_tool_use.web_search }),
    )
  if (r.server_tool_use.web_fetch > 0)
    tools.push(
      t("usage.logs.detail.webFetch", { n: r.server_tool_use.web_fetch }),
    )

  return (
    <TableRow className="hover:bg-transparent">
      <TableCell colSpan={7} className="bg-muted/20 border-t-0 px-3 py-2.5">
        <div className="grid grid-cols-[minmax(0,auto)_minmax(0,1fr)] gap-x-8 gap-y-1.5">
          {/* 成本明细 — the "why is this call expensive" half. 每行 token →
              金额，读一眼就知道这笔钱花在哪。 */}
          <div className="flex flex-col gap-1">
            <div className="text-foreground mb-0.5 text-[11px] font-semibold">
              {t("usage.logs.detail.costTitle")}
            </div>
            <CostLine
              label={t("usage.logs.detail.output")}
              tokens={formatTokens(r.tokens.output)}
              value={formatCost(r.cost.output_usd)}
            />
            <CostLine
              label={t("usage.logs.detail.input")}
              tokens={formatTokens(r.tokens.input)}
              value={formatCost(r.cost.input_usd)}
            />
            <CostLine
              label={t("usage.logs.detail.cacheRead")}
              tokens={formatTokens(r.tokens.cache_read)}
              value={formatCost(r.cost.cache_read_usd)}
            />
            <CostLine
              label={t("usage.logs.detail.cacheCreate")}
              tokens={formatTokens(r.tokens.cache_creation)}
              value={formatCost(r.cost.cache_creation_usd)}
            />
            <CostLine
              label={t("usage.logs.col.cost")}
              tokens={formatTokens(tokenTotal(r))}
              value={formatCost(r.total_cost_usd)}
            />
          </div>
          {/* 其余字段 — identity + context. 两列 grid：标签列等宽，所有行的
              值从同一位置开始（会话/请求 ID 是变长值，inline 布局会对不齐）。
              行距 gap-y-0.5（2px）比左侧金额明细更紧——键值对字段密集，带
              复制按钮的行视觉更重，收紧后上下不再显空。CopyButton 缩到
              size-4 与 12px 字号行高等高（默认 icon-sm 是 size-8，会撑高
              所在行）。会话 ID 与请求 ID 相邻成组（都能复制），随后是
              迭代 / 档位 / 工具 / 计价模型。 */}
          <div className="text-muted-foreground grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-0.5 text-xs">
            {r.session_id ? (
              <>
                <span className="font-medium">
                  {t("usage.logs.detail.session")}
                </span>
                <span className="flex min-w-0 items-center gap-1.5">
                  <Tooltip>
                    <TooltipTrigger
                      render={
                        <span className="truncate font-mono">
                          {r.session_id}
                        </span>
                      }
                    />
                    <TooltipContent>{r.session_id}</TooltipContent>
                  </Tooltip>
                  <CopyButton
                    value={r.session_id}
                    label={t("usage.logs.detail.copy")}
                    className="size-4"
                  />
                </span>
              </>
            ) : null}
            <span className="font-medium">
              {t("usage.logs.detail.requestId")}
            </span>
            <span className="flex min-w-0 items-center gap-1.5">
              <Tooltip>
                <TooltipTrigger
                  render={<span className="truncate font-mono">{r.uuid}</span>}
                />
                <TooltipContent>{r.uuid}</TooltipContent>
              </Tooltip>
              <CopyButton
                value={r.uuid}
                label={t("usage.logs.detail.copy")}
                className="size-4"
              />
            </span>
            {r.iterations > 0 ? (
              <>
                <span className="font-medium">
                  {t("usage.logs.detail.iterations")}
                </span>
                <span className="tabular-nums">{formatInt(r.iterations)}</span>
              </>
            ) : null}
            {r.service_tier ? (
              <>
                <span className="font-medium">
                  {t("usage.logs.detail.serviceTier")}
                </span>
                <span className="font-mono">
                  {/* standard/priority 是 Anthropic API 的档位（Claude Code
                      JSONL 的 usage.service_tier 原样透传）：已知档位显示
                      本地化文案，未知值原样（新档位不崩）。 */}
                  {r.service_tier === "standard"
                    ? t("usage.logs.detail.tier.standard")
                    : r.service_tier === "priority"
                      ? t("usage.logs.detail.tier.priority")
                      : r.service_tier}
                </span>
              </>
            ) : null}
            {tools.length > 0 ? (
              <>
                <span className="font-medium">
                  {t("usage.logs.detail.tools")}
                </span>
                <span>{tools.join(" · ")}</span>
              </>
            ) : null}
            {r.pricing_model && r.pricing_model !== r.model ? (
              <>
                <span className="font-medium">
                  {t("usage.logs.detail.pricingModel")}
                </span>
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <span className="truncate font-mono">
                        {r.pricing_model}
                      </span>
                    }
                  />
                  <TooltipContent>{r.pricing_model}</TooltipContent>
                </Tooltip>
              </>
            ) : null}
          </div>
        </div>
      </TableCell>
    </TableRow>
  )
}
