// 身份卡两态（stats-rail 卡片组置底的一张）：会话态 = 项目 basename + 模型
// badge + 全路径、归属、会话 ID、设备、最近活跃；项目态 = 全路径、subagent
// 归属、最近活跃。两卡共用 KvRow 的对齐骨架——display:contents 让标签/值两
// 格直接成为父级 grid 的行成员，首列 auto 取当前语言下最宽标签，全卡值列共
// 用一条左缘对齐线（标签各自定宽时 Path/Session ID/Device/Last active 的值
// 参差起始，曾被用户否掉）。

import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { CopyButton } from "@/components/copy-button"
import { RelativeTime } from "@/components/relative-time"
import { Badge } from "@/components/ui/badge"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatCount } from "@/lib/format"
import type { SessionRow, SessionStatsRow } from "@/types/generated/bindings"
import { projectBasename } from "../derive"
import { Card } from "./stats-card"

/** 身份卡（会话态，置底）：项目 basename + 全路径、归属、ID、设备、活跃。 */
export function IdentityCard({
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
      {/* 首列 auto＝全卡最宽标签：所有 KvRow 的值列共用一条对齐线。 */}
      <div className="grid grid-cols-[auto_1fr] items-baseline gap-x-2 gap-y-1 text-[11px]">
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
          {/* block flex 而非 inline-flex：inline 级宽度不受父约束，UUID 长
              文本会撑破卡片并把右栏的 overflow-y-auto 逼出横滚；flex 子项
              配 min-w-0 才能真正收缩截断。 */}
          <span className="flex min-w-0 items-center gap-1">
            <code className="min-w-0 flex-1 truncate font-mono">{s.id}</code>
            <CopyButton
              value={s.id}
              label={copyLabel}
              className="size-3.5 shrink-0"
            />
          </span>
        </KvRow>
        <KvRow label={t("sessions.detail.device")}>
          {deviceLabel(s.device_id)}
        </KvRow>
        <KvRow label={t("sessions.detail.lastActive")}>
          <RelativeTime
            ts={s.last_active_at}
            className="tabular-nums"
            side="left"
          />
        </KvRow>
      </div>
    </Card>
  )
}

/** 身份卡（项目态，置底）：全路径、subagent 归属、最近活跃。 */
export function ProjectIdentityCard({
  dir,
  subagents,
  lastActiveAt,
}: {
  dir: string
  subagents: number
  lastActiveAt: string | null
}) {
  const { t } = useTranslation()
  return (
    <Card title={t("sessions.stats.identity")} className="mt-auto">
      <div className="mb-1.5 truncate text-xs font-semibold">
        {dir ? projectBasename(dir) : t("sessions.tree.noProject")}
      </div>
      {/* 首列 auto＝全卡最宽标签：所有 KvRow 的值列共用一条对齐线。 */}
      <div className="grid grid-cols-[auto_1fr] items-baseline gap-x-2 gap-y-1 text-[11px]">
        <KvRow label={t("sessions.stats.idPath")}>
          <Tooltip>
            <TooltipTrigger render={<span className="block w-full truncate" />}>
              {dir || "—"}
            </TooltipTrigger>
            <TooltipContent side="left" className="max-w-sm break-all">
              {dir || "—"}
            </TooltipContent>
          </Tooltip>
        </KvRow>
        <KvRow label={t("sessions.stats.subagent")}>
          {t("sessions.stats.idSubagents", { n: formatCount(subagents) })}
        </KvRow>
        <KvRow label={t("sessions.detail.lastActive")}>
          <RelativeTime
            ts={lastActiveAt}
            className="tabular-nums"
            side="left"
          />
        </KvRow>
      </div>
    </Card>
  )
}

function KvRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    // display:contents——标签/值两格直接成为父级 grid 的行成员。父列定义
    // grid-cols-[auto_1fr]：首列 auto 取当前语言下最宽标签，全卡值列共用一
    // 条左缘对齐线（见文件头注）；标签不折行，值格 min-w-0 保住 UUID 截断。
    <div className="contents">
      <span className="text-muted-foreground whitespace-nowrap">{label}</span>
      <span className="text-foreground min-w-0">{children}</span>
    </div>
  )
}
