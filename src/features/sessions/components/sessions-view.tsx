// Sessions view — 会话管理入口. Two tabs (Local / Favorites), each with its
// own grouping track: Local tab groups by `local_group_id` (device-private),
// Favorites tab groups by `synced_group_id` (git-synced) and shows the source
// device per row. Clicking a row opens the detail Sheet with the transcript.
//
// Pure rendering only — all state, queries, mutations and the optimistic
// favorite / pending-group handling live in useSessionsBrowser (./use-sessions-
// browser). This component owns JSX, styles, i18n and the source-display helper
// (../source-labels). Mirrors library-view.tsx's split.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import { MessagesSquare, Search, Star } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useDistinctModelsQuery } from "@/app/store/api"
import {
  DateRangeChip,
  type DateRangePreset,
} from "@/components/date-range-chip"
import { QueryState } from "@/components/query-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatCost, formatInt, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { SessionRow } from "@/types/generated/bindings"
import type { SessionTab } from "../derive"
import { sessionAgentKind, sessionSourceLabel } from "../source-labels"
import { useSessionsBrowser } from "../use-sessions-browser"
import { GroupCreateDialog } from "./group-create-dialog"
import { GroupSidebar } from "./group-sidebar"
import { SessionDetailSheet } from "./session-detail-sheet"

dayjs.extend(relativeTime)

export function SessionsView() {
  const b = useSessionsBrowser()
  const { t } = useTranslation()
  // Narrow `preview` to a non-null local so the detail-sheet callbacks capture
  // a SessionRow, not SessionRow | null (TS will not narrow across callbacks
  // that read the field later).
  const preview = b.preview

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
      {/* Control row — tabs + filter chips on the left; collect · search on
        the right. Mirrors the request-log ControlBar's chip layout. */}
      <div className="flex flex-wrap items-center gap-2">
        <Tabs value={b.tab} onValueChange={(v) => b.setTab(v as SessionTab)}>
          <TabsList>
            <TabsTrigger value="local">{t("sessions.tab.local")}</TabsTrigger>
            <TabsTrigger value="favorites">
              {t("sessions.tab.favorites")}
            </TabsTrigger>
          </TabsList>
        </Tabs>
        <DateRangeChip
          preset={b.rangePreset}
          fromDay={b.fromDay}
          toDay={b.toDay}
          onPreset={b.setRangePreset}
          onFromDay={b.setFromDay}
          onToDay={b.setToDay}
          presets={RANGE_PRESETS}
          allTimeKey="sessions.filter.allTime"
          dateRangeKey="sessions.filter.dateRange"
          startKey="sessions.filter.start"
          endKey="sessions.filter.end"
        />
        <SourceSelect value={b.source} onChange={b.setSource} />
        <ModelSelect value={b.model} onChange={b.setModel} />
        {b.deviceOptions.length > 0 && b.tab === "favorites" ? (
          <DeviceSelect
            options={b.deviceOptions}
            value={b.deviceScope}
            onChange={b.setDeviceScope}
          />
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          <div className="relative w-56">
            <Search className="text-muted-foreground absolute top-1/2 left-2 size-3.5 -translate-y-1/2" />
            <Input
              value={b.search}
              onChange={(e) => b.setSearch(e.target.value)}
              placeholder={t("sessions.searchPlaceholder")}
              className="h-8 pl-7"
              aria-label={t("sessions.searchPlaceholder")}
            />
          </div>
        </div>
      </div>

      {/* Sidebar + list */}
      <div className="flex min-h-0 flex-1 gap-3">
        <GroupSidebar
          trackGroups={b.trackGroups}
          grouped={b.grouped}
          totalCount={b.totalCount}
          selectedGroupId={b.selectedGroupId}
          onSelect={b.setSelectedGroupId}
          onCreate={b.openCreateGroup}
          onRename={b.renameGroup}
          onDelete={b.deleteGroup}
          onReorder={b.reorderGroups}
          pendingGroup={b.pendingGroup}
          busyGroupId={b.busyGroupId}
          track={b.effectiveTrack}
        />

        <Card className="flex min-h-0 min-w-0 flex-1 flex-col">
          <CardHeader>
            {/* truncate: on a narrow window the card is squeezed beside the
              group sidebar; a long localized title must ellipsize instead of
              wrapping into the sidebar area. */}
            <CardTitle className="truncate">{t("sessions.title")}</CardTitle>
          </CardHeader>
          <CardContent className="flex min-h-0 flex-1 flex-col">
            <QueryState
              isLoading={b.isLoading}
              error={b.error}
              isEmpty={!b.isLoading && b.visibleSessions.length === 0}
              emptyIcon={MessagesSquare}
              emptyLabel={
                b.search ? t("sessions.noMatch") : t("sessions.empty.title")
              }
              emptyDescription={b.search ? undefined : t("sessions.empty.desc")}
            >
              <SessionsTable
                rows={b.visibleSessions}
                effectiveFavorite={b.effectiveFavorite}
                onToggleFavorite={b.toggleFavorite}
                onOpen={b.setPreview}
                showDeviceColumn={b.showDeviceColumn}
                deviceLabel={b.deviceLabel}
              />
            </QueryState>
          </CardContent>
        </Card>
      </div>

      {preview ? (
        <SessionDetailSheet
          session={preview}
          favorited={b.effectiveFavorite(preview)}
          onClose={() => b.setPreview(null)}
          onToggleFavorite={() => b.toggleFavorite(preview)}
          editTitle={b.editTitle}
          titleDraft={b.titleDraft}
          onTitleDraft={b.setTitleDraft}
          onStartTitle={b.startEditTitle}
          onCancelTitle={b.cancelEditTitle}
          onCommitTitle={b.commitEditTitle}
          trackGroups={b.trackGroups}
          currentGroupId={
            b.effectiveTrack === "local"
              ? preview.local_group_id
              : preview.synced_group_id
          }
          onSetGroup={(groupId) => b.setSessionGroup(preview, groupId)}
          transcript={b.transcript}
          transcriptLoading={b.transcriptLoading}
          transcriptError={b.transcriptError}
          onRefreshTranscript={b.refetchTranscript}
          deviceLabel={(id) => b.deviceLabel.get(id) ?? id.slice(0, 8)}
        />
      ) : null}

      <GroupCreateDialog
        open={b.createGroupOpen}
        onClose={() => b.setCreateGroupOpen(false)}
        onCreate={b.createGroup}
        creating={b.pendingGroup !== null}
        track={b.effectiveTrack}
      />
    </div>
  )
}

function SessionsTable({
  rows,
  effectiveFavorite,
  onToggleFavorite,
  onOpen,
  showDeviceColumn,
  deviceLabel,
}: {
  rows: SessionRow[]
  effectiveFavorite: (s: SessionRow) => boolean
  onToggleFavorite: (s: SessionRow) => void
  onOpen: (s: SessionRow) => void
  showDeviceColumn: boolean
  deviceLabel: Map<string, string>
}) {
  const { t } = useTranslation()
  return (
    <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
      {/* table-fixed: column widths come from the header row, so the narrow
          numeric columns (w-20/w-24) are never stretched by extra horizontal
          space — the title column (no explicit width) absorbs the remainder.
          min-w: the fixed columns sum to 856px (incl. the header row's w-10
          star column, the w-40 type column and the w-28 device column);
          below that the auto title column would collapse to ~0 and its
          header text overflows into the type column. The floor keeps the
          title readable and lets the outer overflow-auto scroll horizontally
          instead of squeezing columns into overlap. */}
      <Table className="table-fixed min-w-[58rem]">
        <TableHeader>
          <TableRow>
            <TableHead className="w-10" />
            {/* The title column absorbs the remaining space (keeps the numeric
              columns at their fixed narrow widths when maximized) but caps at
              max-w so an ultra-wide window ellipsizes long titles instead of
              stretching the column indefinitely. */}
            <TableHead className="max-w-[24rem]">
              {t("sessions.col.title")}
            </TableHead>
            <TableHead className="w-40">{t("sessions.col.type")}</TableHead>
            {showDeviceColumn ? (
              <TableHead className="w-28">{t("sessions.col.device")}</TableHead>
            ) : null}
            <TableHead className="w-48">{t("sessions.col.project")}</TableHead>
            <TableHead className="w-24">
              {t("sessions.col.lastActive")}
            </TableHead>
            <TableHead className="w-20 text-right">
              {t("sessions.col.requests")}
            </TableHead>
            <TableHead className="w-24 text-right">
              {t("sessions.col.tokens")}
            </TableHead>
            <TableHead className="w-20 text-right">
              {t("sessions.col.cost")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((s) => {
            const fav = effectiveFavorite(s)
            return (
              <TableRow key={`${s.device_id}/${s.id}`}>
                <TableCell>
                  <Tooltip>
                    <TooltipTrigger
                      render={
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label={
                            fav
                              ? t("sessions.row.unfavorite")
                              : t("sessions.row.favorite")
                          }
                          onClick={(e: React.MouseEvent) => {
                            e.stopPropagation()
                            onToggleFavorite(s)
                          }}
                        />
                      }
                    >
                      <Star
                        className={cn(
                          "size-4",
                          fav
                            ? "fill-accent-brand text-accent-brand"
                            : "text-muted-foreground",
                        )}
                      />
                    </TooltipTrigger>
                    <TooltipContent>
                      {fav
                        ? t("sessions.row.unfavorite")
                        : t("sessions.row.favorite")}
                    </TooltipContent>
                  </Tooltip>
                </TableCell>
                <TableCell>
                  {/* trackCursorAxis: the trigger is the full column width, so
                    a centered tooltip would float over the column's middle
                    (wrong for short titles) — anchor it to the cursor. */}
                  <Tooltip trackCursorAxis="both">
                    <TooltipTrigger
                      render={
                        <button
                          type="button"
                          className="hover:text-accent-brand-strong flex w-full min-w-0 flex-col items-start gap-0.5 text-left"
                          onClick={() => onOpen(s)}
                        />
                      }
                    >
                      <span className="block w-full min-w-0 truncate font-medium">
                        {s.title || t("sessions.untitled")}
                      </span>
                      <span className="text-muted-foreground text-xs">
                        {sessionSourceLabel(s.source)}
                      </span>
                    </TooltipTrigger>
                    <TooltipContent className="max-w-md">
                      {s.title || t("sessions.untitled")}
                    </TooltipContent>
                  </Tooltip>
                </TableCell>
                <TableCell>
                  {(() => {
                    const kind = sessionAgentKind(s.agent_type)
                    // Every non-main row is a subagent, so the badge shows the
                    // bare agent type (e.g. Explore) — no "subagent" prefix.
                    // Long types truncate within the w-40 column instead of
                    // overflowing into the Project column.
                    const label =
                      kind.kind === "main" ? t("sessions.type.main") : kind.type
                    return (
                      <Tooltip>
                        <TooltipTrigger
                          render={
                            <Badge
                              variant="outline"
                              className="max-w-full overflow-hidden font-normal"
                            >
                              <span className="min-w-0 truncate">{label}</span>
                            </Badge>
                          }
                        />
                        <TooltipContent>{label}</TooltipContent>
                      </Tooltip>
                    )
                  })()}
                </TableCell>
                {showDeviceColumn ? (
                  <TableCell>
                    <Badge variant="outline" className="font-normal">
                      {deviceLabel.get(s.device_id) ?? s.device_id.slice(0, 8)}
                    </Badge>
                  </TableCell>
                ) : null}
                <TableCell className="text-muted-foreground text-xs">
                  <Tooltip trackCursorAxis="both">
                    <TooltipTrigger
                      render={<span className="block min-w-0 truncate" />}
                    >
                      {s.project_dir || "—"}
                    </TooltipTrigger>
                    <TooltipContent className="max-w-sm break-all">
                      {s.project_dir || "—"}
                    </TooltipContent>
                  </Tooltip>
                </TableCell>
                <TableCell className="text-muted-foreground text-xs">
                  <span
                    title={dayjs(s.last_active_at).format("YYYY-MM-DD HH:mm")}
                  >
                    {s.last_active_at ? dayjs(s.last_active_at).fromNow() : "—"}
                  </span>
                </TableCell>
                <TableCell className="text-right text-xs tabular-nums">
                  {formatInt(s.request_count)}
                </TableCell>
                <TableCell className="text-right text-xs tabular-nums">
                  {formatTokens(s.total_tokens)}
                </TableCell>
                <TableCell className="text-right text-xs tabular-nums">
                  {formatCost(s.total_cost_usd)}
                </TableCell>
              </TableRow>
            )
          })}
        </TableBody>
      </Table>
    </div>
  )
}

/** "All sources" sentinel for the source dropdown. */
const ALL_SOURCES = "__all__"

/** "All devices" sentinel for the device dropdown. */
const ALL_DEVICES = "__all__"

/** Fixed source options — the sources sessions are collected from. Brand
 *  names are stable, so they live here rather than in i18n (mirrors the usage
 *  view's source-labels); only the "all" option and labels are localized. */
const SOURCE_OPTIONS: string[] = [
  "claude_code",
  "codex_cli",
  "gemini_cli",
  "grok_cli",
  "opencode",
]

function SourceSelect({
  value,
  onChange,
}: {
  value: string
  onChange: (v: string) => void
}) {
  const { t } = useTranslation()
  return (
    <Select
      value={value || ALL_SOURCES}
      onValueChange={(v) => onChange(v === ALL_SOURCES ? "" : (v ?? ""))}
    >
      <SelectTrigger
        className="h-8 w-40"
        aria-label={t("sessions.filter.source")}
      >
        <SelectValue className="min-w-0">
          {(val: string) =>
            val === ALL_SOURCES
              ? t("sessions.filter.allSources")
              : sessionSourceLabel(val)
          }
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL_SOURCES}>
          {t("sessions.filter.allSources")}
        </SelectItem>
        {SOURCE_OPTIONS.map((src) => (
          <SelectItem key={src} value={src}>
            {sessionSourceLabel(src)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

/** "All models" sentinel for the model dropdown. */
const ALL_MODELS = "__all__"

/** Model dropdown — EXISTS semantics (a session that used the model at least
 *  once matches). Options come from the same distinct-models query the request
 *  log uses; the backend EXISTS filter narrows the session list itself. */
function ModelSelect({
  value,
  onChange,
}: {
  value: string
  onChange: (v: string) => void
}) {
  const { t } = useTranslation()
  const { data: models = [] } = useDistinctModelsQuery()
  return (
    <Select
      value={value || ALL_MODELS}
      onValueChange={(v) => onChange(v === ALL_MODELS ? "" : (v ?? ""))}
    >
      <SelectTrigger
        className="h-8 w-40"
        aria-label={t("sessions.filter.model")}
      >
        <SelectValue className="min-w-0">
          {(val: string) =>
            val === ALL_MODELS ? t("sessions.filter.allModels") : val
          }
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL_MODELS}>
          {t("sessions.filter.allModels")}
        </SelectItem>
        {models.map((m) => (
          <SelectItem key={m} value={m}>
            {m}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

const RANGE_PRESETS: DateRangePreset[] = [
  { value: "today", key: "sessions.filter.today" },
  { value: "7d", key: "sessions.filter.last7d" },
  { value: "30d", key: "sessions.filter.last30d" },
  { value: "all", key: "sessions.filter.all" },
]

/** Device dropdown for the Favorites tab — narrows "all devices" to one. */
function DeviceSelect({
  options,
  value,
  onChange,
}: {
  options: { id: string; label: string }[]
  value: string
  onChange: (v: string) => void
}) {
  const { t } = useTranslation()
  return (
    <Select
      value={value || ALL_DEVICES}
      onValueChange={(v) => onChange(v === ALL_DEVICES ? "" : (v ?? ""))}
    >
      <SelectTrigger
        className="h-8 w-40"
        aria-label={t("sessions.filter.device")}
      >
        <SelectValue className="min-w-0">
          {(val: string) =>
            val === ALL_DEVICES
              ? t("sessions.filter.allDevices")
              : (options.find((o) => o.id === val)?.label ?? val)
          }
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL_DEVICES}>
          {t("sessions.filter.allDevices")}
        </SelectItem>
        {options.map((o) => (
          <SelectItem key={o.id} value={o.id}>
            {o.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
