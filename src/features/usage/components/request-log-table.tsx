// Request log table — per-API-call ledger. Columns: Time / App / Billed
// Model / Total / Cost / Stop / Device. The four token buckets and the cost
// breakdown live in the expandable row detail (one row click) instead of five
// extra columns, so the ledger stays scannable; the Source cell shows the
// human-readable tag (`sourceLabel`) with the raw tag in the title tooltip.
// stop_reason renders a localized chip (`stopReasonLabelKey`) with the raw
// English value in the tooltip; calls costing ≥ COST_NOTABLE_THRESHOLD get a
// highlighted cost cell. Rows are time-desc grouped by local day with a day
// separator. Paginated with the shared PaginationBar (ellipsis jumps + per-page
// density); the bar disables while a page refetches. Empty state offers an
// inline 采集 CTA so the user isn't bounced to the command bar to seed the
// first rows.
//
// Pure rendering only — queries / paging / expanded-row state / collect action
// live in useRequestLogBrowser (../use-request-log-browser)，与家族形态对齐
// （视图 = 纯渲染 + use-*-browser hook）。

import { FileText } from "lucide-react"
import { Fragment, type ReactNode } from "react"
import { useTranslation } from "react-i18next"

import type { FilterState } from "@/app/store/slices/filterSlice"
import { collapseTriggerProps } from "@/components/collapse-trigger"
import { CopyButton } from "@/components/copy-button"
import { PaginationBar } from "@/components/pagination-bar"
import { QueryState } from "@/components/query-state"
import { Card, CardContent } from "@/components/ui/card"
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
import { collectLabelKey } from "@/hooks/use-collect-action"
import { deviceLabelOf, useDeviceLabelMap } from "@/lib/device-labels"
import {
  formatCost,
  formatCostPrecise,
  formatDay,
  formatInt,
  formatTime,
  formatTokens,
} from "@/lib/format"
import { totalTokensOf } from "@/lib/token-buckets"
import { cn } from "@/lib/utils"
import type { UsageLogRow } from "@/types/generated/bindings"
import { sourceLabel } from "../source-labels"
import { useRequestLogBrowser } from "../use-request-log-browser"
import { SessionLink } from "./session-link"

export function RequestLogTable({ filter }: { filter: FilterState }) {
  const { t } = useTranslation()
  const deviceLabel = useDeviceLabelMap()
  const b = useRequestLogBrowser(filter)
  const dayGroups = groupRowsByDay(b.rows)

  return (
    <Card className="min-h-0 flex-1">
      <CardContent className="flex min-h-0 flex-1 flex-col">
        <QueryState
          isLoading={b.isLoading}
          error={b.error}
          isEmpty={!b.isLoading && b.rows.length === 0}
          emptyIcon={FileText}
          emptyLabel={t("usage.logs.empty")}
          emptyAction={{
            // 文案 key 与 sidebar 同一归属（collectLabelKey，collecting 态两处
            // 共用）；空闲态注入空态 CTA 自己的「采集本地日志」引导首次入库。
            label: t(
              collectLabelKey(
                b.collecting,
                b.multiDevice,
                "usage.collect.collectLocal",
              ),
            ),
            onClick: b.onCollect,
            disabled: b.collecting,
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
                          expanded={b.expandedId === r.uuid}
                          onToggle={() => b.toggleRow(r.uuid)}
                        />
                        {b.expandedId === r.uuid ? <DetailRow r={r} /> : null}
                      </Fragment>
                    ))}
                  </Fragment>
                ))}
              </TableBody>
            </Table>
          </div>
        </QueryState>

        <PaginationBar
          page={b.page}
          totalPages={b.totalPages}
          total={b.total}
          loading={b.isFetching}
          onPageChange={b.goToPage}
          density={b.density}
        />
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
    // 整行是折叠触发器（点行展开明细）：键盘契约由 collapseTriggerProps 工
    // 厂给出（Enter/Space 切换 + aria-expanded）——tbody 里触发器只能是 tr
    // 本身，故用工厂展开而非 <CollapseTrigger> div。
    <TableRow
      {...collapseTriggerProps({ expanded, onToggle })}
      className={cn("cursor-pointer", expanded && "bg-hover")}
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
          <TooltipTrigger
            render={<span>{formatTokens(totalTokensOf(r))}</span>}
          />
          <TooltipContent>{formatInt(totalTokensOf(r))}</TooltipContent>
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
              render={<span>{deviceLabelOf(deviceLabel, r.device_id)}</span>}
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
              金额，读一眼就知道这笔钱花在哪。逐桶金额常在分厘级，走
              formatCostPrecise（4 位精度），不吃 DSL 指标层恒两位的舍入。 */}
          <div className="flex flex-col gap-1">
            <div className="text-foreground mb-0.5 text-[11px] font-semibold">
              {t("usage.logs.detail.costTitle")}
            </div>
            <CostLine
              label={t("usage.logs.detail.output")}
              tokens={formatTokens(r.tokens.output)}
              value={formatCostPrecise(r.cost.output_usd)}
            />
            <CostLine
              label={t("usage.logs.detail.input")}
              tokens={formatTokens(r.tokens.input)}
              value={formatCostPrecise(r.cost.input_usd)}
            />
            <CostLine
              label={t("usage.logs.detail.cacheRead")}
              tokens={formatTokens(r.tokens.cache_read)}
              value={formatCostPrecise(r.cost.cache_read_usd)}
            />
            <CostLine
              label={t("usage.logs.detail.cacheCreate")}
              tokens={formatTokens(r.tokens.cache_creation)}
              value={formatCostPrecise(r.cost.cache_creation_usd)}
            />
            <CostLine
              label={t("usage.logs.col.cost")}
              tokens={formatTokens(totalTokensOf(r))}
              value={formatCostPrecise(r.total_cost_usd)}
            />
          </div>
          {/* 其余字段 — identity + context. 两列 grid：标签列等宽，所有行的
              值从同一位置开始（会话/请求 ID 是变长值，inline 布局会对不齐）。
              行距压到最紧——leading-none（12px 文本不再撑 16px 行框）+
              gap-y-0，键值对字段贴成一块密集的 spec 区，块与块之间不再
              显空。带复制按钮的行由 CopyButton（size-4，16px）决定行高。
              会话 ID 与请求 ID 相邻成组（都能复制），随后是
              迭代 / 档位 / 工具 / 计价模型。 */}
          <div className="text-muted-foreground grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-0 text-xs leading-none">
            {r.session_id ? (
              <>
                <span className="font-medium">
                  {t("usage.logs.detail.session")}
                </span>
                <span className="flex min-w-0 items-center gap-1.5">
                  {/* session_id 解析为会话标题 + 点击跳转（usage→sessions
                      跨域通道）；未命中退回裸 id，复制按钮仍拷 id。 */}
                  <SessionLink
                    sessionId={r.session_id}
                    deviceId={r.device_id}
                  />
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
