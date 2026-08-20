// 右统计卡栏 —— 四栏工作台第四栏（定稿 §3）。口径 tab「按项目 / 按会话」跟随
// 选中对象（选中会话时默认按会话，容器选中时默认按项目，用户可切换）。
//
// 按会话 = 4 张卡：用量（Token 四桶条形 + 总量 + 缓存命中率〔四桶派生结论，
// 仅此一处〕+ 成本）、活动（轮次·含工具轮数 / 时长 / 请求数 / 消息数）、
// 按模型占比、身份（项目 basename+全路径、会话 ID、最近活跃，置底）。
// 按项目与中栏项目统计头去重：选中项目时仅留中栏没有的（模型、耗时分布），
// 未选中时展示全量聚合的用量 + 活动 + 模型 + 耗时。
//
// 窄容器（< 48rem，768 档）整栏折叠为「统计」浮动按钮开抽屉——卡片组同一份
// 渲染进右栏与抽屉两处。数据全部来自 useSessionsBrowser 的 selection-free
// session_stats 读（aggregateStats 纯聚合），无第二条统计路径。

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { BarChart3 } from "lucide-react"
import { type ReactNode, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { CopyButton } from "@/components/copy-button"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Sheet, SheetContent } from "@/components/ui/sheet"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  formatCost,
  formatCount,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTokens,
} from "@/lib/format"
import { cn } from "@/lib/utils"
import type {
  SessionMessage,
  SessionRow,
  SessionStatsRow,
} from "@/types/generated/bindings"
import { groupConversation } from "../conversation"
import {
  type ModelShare,
  projectBasename,
  type StatsAggregate,
  sessionSpan,
  spanLabelKey,
} from "../derive"
import { turnAnchors } from "../turn-nav"

dayjs.extend(relativeTime)

/** 四桶条形的分段色 —— 与 usage 域图表同一组 B 级语义 token。 */
const BUCKET_COLORS = {
  input: "var(--chart-input)",
  output: "var(--chart-output)",
  cache_creation: "var(--chart-cache-create)",
  cache_read: "var(--chart-cache-read)",
} as const

export type StatsRailScope = "project" | "session"

export function StatsRail({
  scope,
  onScopeChange,
  scopeLabel,
  aggregate,
  session,
  sessionStats,
  transcript,
  transcriptLoading,
  deviceLabel,
  projectSelected,
}: {
  scope: StatsRailScope
  onScopeChange: (s: StatsRailScope) => void
  /** 口径对象名（会话标题 / 项目 basename / 组名 / 「全部」）。 */
  scopeLabel: string
  /** 选中容器的聚合（未选中 = 全量）。 */
  aggregate: StatsAggregate
  /** 选中会话（按会话口径的数据源；null = 聚合回退）。 */
  session: SessionRow | null
  sessionStats: SessionStatsRow | null
  transcript: SessionMessage[]
  transcriptLoading: boolean
  deviceLabel: (id: string) => string
  /** 项目选中（中栏已示统计头 → 右栏去重，仅留模型/耗时）。 */
  projectSelected: boolean
}) {
  const { t } = useTranslation()
  const sessionScope = scope === "session" && session !== null
  const cards = sessionScope ? (
    <SessionCards
      session={session}
      stats={sessionStats}
      aggregate={aggregate}
      transcript={transcript}
      transcriptLoading={transcriptLoading}
      deviceLabel={deviceLabel}
    />
  ) : (
    <AggregateCards aggregate={aggregate} projectSelected={projectSelected} />
  )
  const header = (
    <div className="flex items-center gap-2">
      <Tabs
        value={scope}
        onValueChange={(v) => onScopeChange(v as StatsRailScope)}
      >
        <TabsList>
          <TabsTrigger value="project" className="px-2 text-xs">
            {t("sessions.stats.byProject")}
          </TabsTrigger>
          <TabsTrigger value="session" className="px-2 text-xs">
            {t("sessions.stats.bySession")}
          </TabsTrigger>
        </TabsList>
      </Tabs>
      <Tooltip>
        <TooltipTrigger
          render={
            <span className="min-w-0 flex-1 truncate text-right text-xs" />
          }
        >
          <span className="text-muted-foreground">
            {t("sessions.stats.scope")}{" "}
            <span className="text-accent-brand-strong font-medium">
              {scopeLabel}
            </span>
          </span>
        </TooltipTrigger>
        <TooltipContent side="left">{scopeLabel}</TooltipContent>
      </Tooltip>
    </div>
  )

  return (
    <>
      {/* 右栏本体：48rem 容器以下整栏让位（抽屉接管）。 */}
      <aside className="border-border bg-card hidden min-h-0 w-72 shrink-0 flex-col gap-2 rounded-lg border p-2 @[48rem]:flex @[64rem]:w-80">
        {header}
        <div className="scrollbar-none min-h-0 flex-1 overflow-y-auto pr-0.5">
          <div className="grid grid-cols-2 gap-2 @[64rem]:grid-cols-1">
            {cards}
          </div>
        </div>
      </aside>
      {/* 窄容器：浮动按钮 + 抽屉（卡片同一渲染）。 */}
      <StatsDrawer header={header}>{cards}</StatsDrawer>
    </>
  )
}

/** 768 档的「统计」浮动按钮 + 抽屉。 */
function StatsDrawer({
  header,
  children,
}: {
  header: ReactNode
  children: ReactNode
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button
        size="sm"
        onClick={() => setOpen(true)}
        aria-label={t("sessions.stats.open")}
        className="fixed right-4 bottom-16 z-40 h-8 rounded-full px-3 text-xs shadow-lg @[48rem]:hidden"
      >
        <BarChart3 className="size-3.5" />
        {t("sessions.stats.open")}
      </Button>
      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-80 gap-2 p-2 sm:max-w-[85vw]">
          {header}
          <div className="scrollbar-none min-h-0 flex-1 overflow-y-auto pr-0.5">
            <div className="grid grid-cols-1 gap-2">{children}</div>
          </div>
        </SheetContent>
      </Sheet>
    </>
  )
}

// ------------------------------------------------------------- 卡片组 ----

/** 按会话 4 卡：详情头瘦身（定稿 §2）后，身份与统计全部落在这里。 */
function SessionCards({
  session: s,
  stats,
  aggregate,
  transcript,
  transcriptLoading,
  deviceLabel,
}: {
  session: SessionRow
  stats: SessionStatsRow | null
  aggregate: StatsAggregate
  transcript: SessionMessage[]
  transcriptLoading: boolean
  deviceLabel: (id: string) => string
}) {
  const { t } = useTranslation()
  // 选中会话但统计行尚未到位（跨页回退行不在当前宇宙）→ 用容器聚合兜底，
  // 卡片不空转。
  const agg = stats
    ? {
        tokens: {
          input: stats.input_tokens,
          output: stats.output_tokens,
          cache_creation: stats.cache_creation_tokens,
          cache_read: stats.cache_read_tokens,
        },
        hitRate: stats.cache_hit_rate,
        cost: stats.total_cost_usd ?? 0,
        models: stats.models.map<ModelShare>((m) => ({
          model: m.model,
          tokens: m.tokens,
          sessions: 1,
        })),
      }
    : {
        tokens: aggregate.tokens,
        hitRate: aggregate.hitRate,
        cost: aggregate.cost,
        models: aggregate.models,
      }
  // 轮次结构（#86 的生产派生）：轮数 = 用户消息锚点数；含工具轮数 = 任一
  // 节点挂工具块的轮数。加载中显示占位。
  const turns = useMemo(() => turnAnchors(transcript).length, [transcript])
  const toolTurns = useMemo(
    () =>
      groupConversation(transcript).filter((turn) =>
        turn.nodes.some((n) => n.tools.length > 0),
      ).length,
    [transcript],
  )
  // 会话时长：started_at 缺失时用首条消息时间兜底（与详情口径一致）。
  const started = s.started_at || transcript[0]?.ts || null
  const span = sessionSpan(
    started && s.last_active_at
      ? dayjs(s.last_active_at).diff(dayjs(started))
      : null,
  )
  const spanKey = spanLabelKey(span)
  const spanLabel = spanKey ? t(spanKey.key, spanKey.vars) : "—"

  return (
    <>
      <UsageCard
        tokens={agg.tokens}
        hitRate={agg.hitRate}
        cost={agg.cost}
        className="col-span-2 @[64rem]:col-span-1"
      />
      <Card title={t("sessions.stats.activity")}>
        <ActGrid
          cells={[
            [
              transcriptLoading ? "—" : formatCount(turns),
              t("sessions.stats.turns", {
                n: formatCount(transcriptLoading ? 0 : toolTurns),
              }),
            ],
            [spanLabel, t("sessions.detail.duration")],
            [formatCount(s.request_count), t("sessions.detail.requests")],
            [
              transcriptLoading ? "—" : formatCount(transcript.length),
              t("sessions.detail.messages"),
            ],
          ]}
        />
      </Card>
      <ModelCard models={agg.models} total={sumTokens(agg.tokens)} />
      <IdentityCard session={s} stats={stats} deviceLabel={deviceLabel} />
    </>
  )
}

/** 按项目卡组：项目选中时与中栏统计头去重（仅模型 + 耗时），否则全量。 */
function AggregateCards({
  aggregate,
  projectSelected,
}: {
  aggregate: StatsAggregate
  projectSelected: boolean
}) {
  const { t } = useTranslation()
  const totalSpanKey = spanLabelKey(sessionSpan(aggregate.totalSpanMs))
  const totalSpanLabel = totalSpanKey
    ? t(totalSpanKey.key, totalSpanKey.vars)
    : "—"
  return (
    <>
      {projectSelected ? (
        <p className="text-muted-foreground/70 col-span-2 @[64rem]:col-span-1 px-1 text-[11px] leading-relaxed">
          {t("sessions.stats.dedupNote")}
        </p>
      ) : (
        <>
          <UsageCard
            tokens={aggregate.tokens}
            hitRate={aggregate.hitRate}
            cost={aggregate.cost}
            className="col-span-2 @[64rem]:col-span-1"
          />
          <Card title={t("sessions.stats.activity")}>
            <ActGrid
              cells={[
                [formatCount(aggregate.sessions), t("sessions.stats.sessions")],
                [totalSpanLabel, t("sessions.stats.totalDuration")],
                [
                  formatCount(aggregate.requests),
                  t("sessions.detail.requests"),
                ],
                [formatCount(aggregate.messages), t("sessions.stats.messages")],
              ]}
            />
          </Card>
        </>
      )}
      <ModelCard
        models={aggregate.models}
        total={sumTokens(aggregate.tokens)}
        showSessionCounts
      />
      <DurationCard aggregate={aggregate} />
    </>
  )
}

// ------------------------------------------------------------- 卡片 ----

/** 卡片外壳：小标题 + 任意体。 */
function Card({
  title,
  children,
  className,
}: {
  title: string
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cn(
        "border-border bg-card/60 rounded-lg border p-2.5",
        className,
      )}
    >
      <h4 className="text-xs font-semibold">{title}</h4>
      <div className="mt-2">{children}</div>
    </section>
  )
}

/** 用量卡：四桶条形 + 图例 + 脚注（命中率/成本 = 四桶派生结论，同卡呈现）。 */
function UsageCard({
  tokens,
  hitRate,
  cost,
  className,
}: {
  tokens: StatsAggregate["tokens"]
  hitRate: number | null
  cost: number | null
  className?: string
}) {
  const { t } = useTranslation()
  const total = sumTokens(tokens)
  const segments = [
    {
      label: t("sessions.stats.bucket.input"),
      value: tokens.input,
      color: BUCKET_COLORS.input,
    },
    {
      label: t("sessions.stats.bucket.output"),
      value: tokens.output,
      color: BUCKET_COLORS.output,
    },
    {
      label: t("sessions.stats.bucket.cacheCreation"),
      value: tokens.cache_creation,
      color: BUCKET_COLORS.cache_creation,
    },
    {
      label: t("sessions.stats.bucket.cacheRead"),
      value: tokens.cache_read,
      color: BUCKET_COLORS.cache_read,
    },
  ]
  return (
    <Card title={t("sessions.stats.usage")} className={className}>
      <p className="text-muted-foreground text-[10px] tabular-nums">
        {t("sessions.stats.usageTotal", { n: formatTokens(total) })}
      </p>
      <div className="bg-muted mt-1.5 flex h-2 gap-px overflow-hidden rounded-sm">
        {total > 0
          ? segments.map((seg) => (
              <span
                key={seg.label}
                // DSL 段 tooltip：标签 数量 · 占比。
                title={formatMetricSeg(
                  seg.label,
                  formatTokens(seg.value),
                  seg.value / total,
                )}
                style={{
                  width: `${(seg.value / total) * 100}%`,
                  background: seg.color,
                }}
                className="block h-full min-w-[2px]"
              />
            ))
          : null}
      </div>
      <div className="mt-2 flex flex-col gap-1 text-[11px]">
        {segments.map((seg) => (
          <div key={seg.label} className="flex items-center gap-1.5">
            <span
              className="size-2 shrink-0 rounded-[2px]"
              style={{ background: seg.color }}
            />
            <span className="text-muted-foreground min-w-0 flex-1 truncate">
              {seg.label}
            </span>
            <span className="tabular-nums">{formatTokens(seg.value)}</span>
            <span className="text-muted-foreground/70 w-11 text-right tabular-nums">
              {/* DSL：占比恒一位小数（与正文精度一致）。 */}
              {total > 0 ? formatPct(seg.value / total) : "—"}
            </span>
          </div>
        ))}
      </div>
      <div className="border-border mt-2 grid grid-cols-2 gap-2 border-t pt-2">
        <div>
          <div className="text-muted-foreground text-[10px]">
            {t("sessions.stats.hitRate")}
          </div>
          <div
            className="text-[15px] font-semibold tabular-nums"
            style={{ color: BUCKET_COLORS.cache_read }}
          >
            {formatPct(hitRate)}
          </div>
          <div className="text-muted-foreground/60 mt-0.5 text-[9.5px] leading-tight">
            {t("sessions.stats.hitRateFormula")}
          </div>
        </div>
        <div>
          <div className="text-muted-foreground text-[10px]">
            {t("sessions.detail.cost")}
          </div>
          <div className="text-accent-brand-strong text-[15px] font-semibold tabular-nums">
            {formatCost(cost)}
          </div>
          <div className="text-muted-foreground/60 mt-0.5 text-[9.5px] leading-tight">
            {t("sessions.stats.costNote")}
          </div>
        </div>
      </div>
    </Card>
  )
}

/** 活动卡 2×2 数字格。 */
function ActGrid({ cells }: { cells: [string, string][] }) {
  return (
    <div className="grid grid-cols-2 gap-x-3 gap-y-2.5">
      {cells.map(([n, k]) => (
        <div key={k}>
          <div className="text-[15px] font-semibold tabular-nums">{n}</div>
          <div className="text-muted-foreground mt-0.5 text-[10px]">{k}</div>
        </div>
      ))}
    </div>
  )
}

/** 模型占比卡：灰阶条 + 直接标注（占比不依赖颜色表达）。 */
function ModelCard({
  models,
  total,
  showSessionCounts,
}: {
  models: ModelShare[]
  total: number
  showSessionCounts?: boolean
}) {
  const { t } = useTranslation()
  const shades = ["bg-foreground/70", "bg-foreground/40", "bg-foreground/20"]
  return (
    <Card title={t("sessions.stats.byModel")}>
      {models.length === 0 ? (
        <p className="text-muted-foreground py-3 text-center text-[11px]">
          {t("sessions.stats.noModels")}
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          {models.map((m, i) => (
            <div key={m.model}>
              <div className="flex items-baseline gap-2 text-[11px]">
                <span className="min-w-0 flex-1 truncate font-mono">
                  {m.model}
                </span>
                {/* DSL：模型名即标签，主值 数量 · 占比（usage 模型分布行同款）。 */}
                <span className="text-muted-foreground shrink-0 tabular-nums">
                  {formatSegValue(
                    formatTokens(m.tokens),
                    total > 0 ? m.tokens / total : null,
                  )}
                </span>
              </div>
              <div className="bg-muted mt-1 h-1.5 overflow-hidden rounded-sm">
                <span
                  className={cn(
                    "block h-full min-w-[2px] rounded-sm",
                    shades[i % shades.length],
                  )}
                  style={{
                    width: total > 0 ? `${(m.tokens / total) * 100}%` : 0,
                  }}
                />
              </div>
              {showSessionCounts ? (
                <div className="text-muted-foreground/60 mt-0.5 text-[10px] tabular-nums">
                  {t("sessions.stats.modelSessions", {
                    n: formatCount(m.sessions),
                  })}
                </div>
              ) : null}
            </div>
          ))}
        </div>
      )}
    </Card>
  )
}

/** 耗时分布卡（仅聚合口径）：四档条形；单会话时其所在档高亮。 */
function DurationCard({ aggregate }: { aggregate: StatsAggregate }) {
  const { t } = useTranslation()
  const labels = [
    t("sessions.stats.dur.lt15"),
    t("sessions.stats.dur.m15to60"),
    t("sessions.stats.dur.h1to3"),
    t("sessions.stats.dur.gt3"),
  ]
  const max = Math.max(...aggregate.durationBuckets, 1)
  const single = aggregate.sessions === 1
  const highlight = single
    ? aggregate.durationBuckets.findIndex((n) => n > 0)
    : -1
  return (
    <Card title={t("sessions.stats.durationDist")}>
      <div className="flex flex-col gap-1.5">
        {aggregate.durationBuckets.map((n, i) => (
          <div
            key={labels[i]}
            className="grid grid-cols-[64px_1fr_24px] items-center gap-2 text-[11px]"
          >
            <span className="text-muted-foreground truncate">{labels[i]}</span>
            <span className="bg-muted h-1.5 overflow-hidden rounded-sm">
              <span
                className={cn(
                  "block h-full rounded-sm",
                  i === highlight ? "bg-accent-brand" : "bg-foreground/40",
                )}
                style={{ width: `${(n / max) * 100}%` }}
              />
            </span>
            <span className="text-right tabular-nums">{n}</span>
          </div>
        ))}
      </div>
    </Card>
  )
}

/** 身份卡（按会话口径，置底）：项目 basename + 全路径、归属、ID、设备、活跃。 */
function IdentityCard({
  session: s,
  stats,
  deviceLabel,
}: {
  session: SessionRow
  stats: SessionStatsRow | null
  deviceLabel: (id: string) => string
}) {
  const { t } = useTranslation()
  const copyLabel = t("sessions.detail.copySessionId")
  const model = stats?.models[0]?.model
  return (
    <Card title={t("sessions.stats.identity")} className="mt-auto">
      <div className="mb-1.5 flex min-w-0 items-center gap-1.5">
        <span className="truncate text-xs font-semibold">
          {s.project_dir
            ? projectBasename(s.project_dir)
            : t("sessions.tree.noProject")}
        </span>
        {model ? (
          <Badge
            variant="secondary"
            className="h-4 px-1.5 font-mono text-[10px] leading-none"
          >
            {model}
          </Badge>
        ) : null}
      </div>
      <div className="flex flex-col gap-1 text-[11px]">
        <KvRow label={t("sessions.stats.idPath")}>
          <Tooltip>
            <TooltipTrigger render={<span className="block w-full truncate" />}>
              {s.project_dir || "—"}
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-sm break-all">
              {s.project_dir || "—"}
            </TooltipContent>
          </Tooltip>
        </KvRow>
        {s.agent_type ? (
          <KvRow label={t("sessions.stats.idAgent")}>
            {t("sessions.stats.idAgentValue", { type: s.agent_type })}
          </KvRow>
        ) : null}
        <KvRow label={t("sessions.detail.sessionId")}>
          <span className="inline-flex min-w-0 items-center gap-1">
            <code className="truncate font-mono">{s.id}</code>
            <CopyButton value={s.id} label={copyLabel} className="size-3.5" />
          </span>
        </KvRow>
        <KvRow label={t("sessions.detail.device")}>
          {deviceLabel(s.device_id)}
        </KvRow>
        <KvRow label={t("sessions.detail.lastActive")}>
          {s.last_active_at ? (
            <Tooltip>
              <TooltipTrigger render={<span className="tabular-nums" />}>
                {dayjs(s.last_active_at).fromNow()}
              </TooltipTrigger>
              <TooltipContent side="left">
                {dayjs(s.last_active_at).format("YYYY-MM-DD HH:mm")}
              </TooltipContent>
            </Tooltip>
          ) : (
            "—"
          )}
        </KvRow>
      </div>
    </Card>
  )
}

function KvRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex min-w-0 gap-2">
      <span className="text-muted-foreground w-12 shrink-0">{label}</span>
      <span className="text-foreground min-w-0 flex-1">{children}</span>
    </div>
  )
}

function sumTokens(t: StatsAggregate["tokens"]): number {
  return t.input + t.output + t.cache_creation + t.cache_read
}
