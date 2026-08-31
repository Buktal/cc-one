// 统计卡片组 —— stats-rail 右栏与窄容器浮卡共用的同一份渲染（口径派生只有
// 一处，StatsCards 一个分派口）。三态：
//
// - 会话态（打开会话）= 4 张卡：用量（Token 四桶构成 + 总量 + 缓存命中率
//   〔四桶派生结论，仅此一处〕+ 成本）、活动（轮次·含工具轮数 / 时长 /
//   请求数 / 消息数）、按模型占比、身份（stats-identity-cards，置底）。
// - 项目态（选中项目 / 未选容器）= 用量 + 活动 + 模型（+ 身份卡：全路径、
//   subagent 归属、最近活跃）。
// - 分组态（选中分组/未分组）= 轻量汇总一张卡：会话数 + 四桶 + 命中率 +
//   成本，不过度展开（验收清单指定）。

import dayjs from "dayjs"
import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import {
  BucketComposition,
  type CompositionSegment,
} from "@/components/bucket-composition"
import {
  formatCost,
  formatCount,
  formatMetricSeg,
  formatPct,
  formatSegValue,
  formatTokens,
  spanLabelKey,
  spanParts,
} from "@/lib/format"
import {
  BUCKET_COLOR,
  BUCKET_DISPLAY,
  sumBuckets,
  tokenBuckets,
} from "@/lib/token-buckets"
import { cn } from "@/lib/utils"
import type {
  SessionMessage,
  SessionRow,
  SessionStatsRow,
} from "@/types/generated/bindings"
import {
  groupConversation,
  toolTurnCount,
  userTurnCount,
} from "../conversation"
import {
  type ModelShare,
  type StatsAggregate,
  sessionStartedAt,
} from "../derive"
import { Card } from "./stats-card"
import { IdentityCard, ProjectIdentityCard } from "./stats-identity-cards"
import type { StatsData } from "./stats-rail"

/** 卡片组——口径派生（会话 > 分组 > 项目），两种载体同一渲染（无第二份分支）。 */
export function StatsCards(data: StatsData) {
  return data.session !== null ? (
    <SessionCards
      session={data.session}
      stats={data.sessionStats}
      aggregate={data.aggregate}
      transcript={data.transcript}
      transcriptLoading={data.transcriptLoading}
      deviceLabel={data.deviceLabel}
    />
  ) : data.scopeTag === "group" ? (
    <GroupSummaryCard aggregate={data.aggregate} />
  ) : (
    <AggregateCards
      aggregate={data.aggregate}
      projectIdentity={data.projectIdentity}
    />
  )
}

// ------------------------------------------------------------- 卡片组 ----

/** 会话态 4 卡：详情头瘦身后，身份与统计全部落在这里。 */
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
  // 选中会话但统计行尚未到位（跨页回退行不在当前宇宙）→ 用容器聚合兜底
  // （containerStatsRows 的会话态切片 = 整份宇宙读数），卡片不空转。
  const agg = stats
    ? {
        tokens: tokenBuckets(stats),
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
  // 轮次结构（#86 的生产派生）：两个计数都是 conversation.ts 的已测规则，
  // 一次 groupConversation 切片喂两个读端。加载中显示占位。
  const turns = useMemo(() => groupConversation(transcript), [transcript])
  const turnCount = useMemo(() => userTurnCount(turns), [turns])
  const toolTurns = useMemo(() => toolTurnCount(turns), [turns])
  // 会话起点：started_at 缺采时用首条消息时间兜底（sessionStartedAt——口径
  // 归属 derive，不再靠注释声明「与详情一致」）。
  const started = useMemo(
    () => sessionStartedAt(s, transcript),
    [s, transcript],
  )
  const span = spanParts(
    started && s.last_active_at
      ? dayjs(s.last_active_at).diff(dayjs(started))
      : null,
  )
  const spanKey = spanLabelKey(span)
  const spanLabel = spanKey ? t(spanKey.key, spanKey.vars) : "—"

  return (
    <>
      <UsageCard tokens={agg.tokens} hitRate={agg.hitRate} cost={agg.cost} />
      <Card title={t("sessions.stats.activity")}>
        <ActGrid
          cells={[
            [
              transcriptLoading ? "—" : formatCount(turnCount),
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
      <ModelCard models={agg.models} total={sumBuckets(agg.tokens)} />
      <IdentityCard session={s} stats={stats} deviceLabel={deviceLabel} />
    </>
  )
}

/** 项目/全量态卡组：用量 + 活动 + 模型；项目态再加身份卡（置底）。 */
function AggregateCards({
  aggregate,
  projectIdentity,
}: {
  aggregate: StatsAggregate
  projectIdentity: { dir: string; subagents: number } | null
}) {
  const { t } = useTranslation()
  const totalSpanKey = spanLabelKey(spanParts(aggregate.totalSpanMs))
  const totalSpanLabel = totalSpanKey
    ? t(totalSpanKey.key, totalSpanKey.vars)
    : "—"
  return (
    <>
      <UsageCard
        tokens={aggregate.tokens}
        hitRate={aggregate.hitRate}
        cost={aggregate.cost}
      />
      <Card title={t("sessions.stats.activity")}>
        <ActGrid
          cells={[
            [formatCount(aggregate.sessions), t("sessions.stats.sessions")],
            [totalSpanLabel, t("sessions.stats.totalDuration")],
            [formatCount(aggregate.requests), t("sessions.detail.requests")],
            [formatCount(aggregate.messages), t("sessions.detail.messages")],
          ]}
        />
      </Card>
      <ModelCard
        models={aggregate.models}
        total={sumBuckets(aggregate.tokens)}
        showSessionCounts
      />
      {projectIdentity ? (
        <ProjectIdentityCard
          dir={projectIdentity.dir}
          subagents={projectIdentity.subagents}
          lastActiveAt={aggregate.lastActiveAt}
        />
      ) : null}
    </>
  )
}

/** 分组态轻量汇总（验收指定口径：会话数 + 四桶 + 命中率 + 成本）。 */
function GroupSummaryCard({ aggregate }: { aggregate: StatsAggregate }) {
  const { t } = useTranslation()
  return (
    <Card title={t("sessions.stats.groupSummary")}>
      {/* DSL 段：会话数 N。 */}
      <div className="text-[13px] tabular-nums">
        {formatMetricSeg(
          t("sessions.stats.sessions"),
          formatCount(aggregate.sessions),
        )}
      </div>
      <UsageBody
        tokens={aggregate.tokens}
        hitRate={aggregate.hitRate}
        cost={aggregate.cost}
        className="border-border mt-2 border-t pt-2"
      />
    </Card>
  )
}

// ------------------------------------------------------------- 卡片 ----

/** 用量卡：四桶构成 + 图例 + 脚注（命中率/成本 = 四桶派生结论，同卡呈现）。 */
function UsageCard({
  tokens,
  hitRate,
  cost,
}: {
  tokens: StatsAggregate["tokens"]
  hitRate: number | null
  cost: number | null
}) {
  const { t } = useTranslation()
  return (
    <Card title={t("sessions.stats.usage")}>
      <UsageBody tokens={tokens} hitRate={hitRate} cost={cost} />
    </Card>
  )
}

/** 用量卡体：总量行 + 四桶构成（BucketComposition 原语，会话域只注入文案键、
 *  值列+独立占比列与条上 tooltip）+ 命中率/成本脚注——用量卡与分组轻量汇总
 *  共享的唯一实现。 */
function UsageBody({
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
  const total = sumBuckets(tokens)
  // 展示名册 BUCKET_DISPLAY（lib/token-buckets）的 sessions 域投影：色/序与
  // usage 域图表同一契约，文案走本域键（前缀在此拼接，名册只持共用尾段）。
  const segments: CompositionSegment[] = BUCKET_DISPLAY.map((b) => ({
    key: b.bucket,
    label: t(`sessions.stats.bucket.${b.suffix}`),
    value: tokens[b.bucket],
    color: b.cssVar,
  }))
  return (
    <div className={className}>
      <p className="text-muted-foreground text-[10px] tabular-nums">
        {t("sessions.stats.usageTotal", { n: formatTokens(total) })}
      </p>
      <div className="mt-1.5">
        <BucketComposition
          segments={segments}
          total={total}
          compact
          segmentTitle={(seg, share) =>
            // DSL 段 tooltip：标签 数量 · 占比。
            formatMetricSeg(seg.label, formatTokens(seg.value), share)
          }
          renderValue={(value, share) => (
            <>
              <span>{formatTokens(value)}</span>
              <span className="text-muted-foreground/70 w-11 text-right">
                {/* DSL：占比恒一位小数（与正文精度一致）；总量 0 → 空档 —。 */}
                {share == null ? "—" : formatPct(share)}
              </span>
            </>
          )}
        />
      </div>
      <div className="border-border mt-2 grid grid-cols-2 gap-2 border-t pt-2">
        <div>
          <div className="text-muted-foreground text-[10px]">
            {t("sessions.stats.hitRate")}
          </div>
          <div
            className="text-[15px] font-semibold tabular-nums"
            style={{ color: BUCKET_COLOR.cache_read }}
          >
            {formatPct(hitRate)}
          </div>
        </div>
        <div>
          <div className="text-muted-foreground text-[10px]">
            {t("sessions.detail.cost")}
          </div>
          <div className="text-accent-brand-strong text-[15px] font-semibold tabular-nums">
            {formatCost(cost)}
          </div>
        </div>
      </div>
    </div>
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
