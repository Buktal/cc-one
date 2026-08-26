// Library view — per-device, git-mediated cloud storage. Drag files / dirs in
// to upload (= push to the sync repo); drill into directories (the same surface
// at every depth — drag-in, export, single-file download all work inside);
// preview a file in the webview; export to a path you choose. cc one never
// writes into an AI tool's own config dir.
//
// Pure rendering only — all state, queries, mutations and navigation live in
// useLibraryBrowser (./use-library-browser). This component owns JSX, styles,
// i18n, and the pure display helper (kindIcon).

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import {
  ArrowUp,
  Check,
  ChevronRight,
  Download,
  FilePlus,
  Folder,
  Loader2,
  Pencil,
  Search,
  Trash2,
  X,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState } from "@/components/empty-state"
import { FilterSelect } from "@/components/filter-select"
import { PaginationBar } from "@/components/pagination-bar"
import { QueryState } from "@/components/query-state"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
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
import { deviceOptionLabel } from "@/features/usage/use-device-options"
import { useConfirmDelete } from "@/hooks/use-confirm-delete"
import { formatSize } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { LibraryEntry } from "@/types/generated/bindings"
import { kindIcon } from "../kind-icon"
import { useLibraryBrowser } from "../use-library-browser"
import { PreviewSheet } from "./preview-sheet"
import { UploadDialog } from "./upload-dialog"

dayjs.extend(relativeTime)

export function LibraryView() {
  const { t } = useTranslation()
  const {
    entries,
    totalCount,
    page,
    totalPages,
    goToPage,
    isLoading,
    scanError,
    refetchScan,
    deviceOptions,
    deviceScope,
    setDeviceScope,
    subpath,
    atRoot,
    showDevice,
    breadcrumb,
    search,
    setSearch,
    dragging,
    pendingPaths,
    clearPendingPaths,
    onAddFiles,
    renaming,
    renameVal,
    setRenameVal,
    cancelRename,
    busyRelPath,
    drill,
    goUp,
    onExport,
    onDelete: deleteEntry,
    startRename,
    commitRename,
    preview,
    setPreview,
  } = useLibraryBrowser()

  // 删除确认（busy / 关闭时序收敛在 useConfirmDelete）。先关再删：行内已有
  // busyRelPath spinner，closeFirst 模式确认后立刻关框、由行级 busy 接管。
  const confirmDelete = useConfirmDelete<LibraryEntry>({
    holdOpen: false,
    onDelete: async (entry) => {
      await deleteEntry(entry)
      return true
    },
  })

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      {/* Toolbar in a fixed element order: navigation (up / device picker /
        breadcrumb) on the left, search + add pinned right as one group —
        on a narrow container the group drops to its own right-aligned line
        instead of scattering between the breadcrumb crumbs. */}
      <div className="@container flex flex-wrap items-center gap-2">
        {!atRoot ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="outline"
                  size="icon-sm"
                  aria-label={t("library.up")}
                  onClick={goUp}
                />
              }
            >
              <ArrowUp />
            </TooltipTrigger>
            <TooltipContent>{t("library.up")}</TooltipContent>
          </Tooltip>
        ) : null}

        {atRoot ? (
          /* 宽度策略（自适应 + 上限）收敛在 FilterSelect 本体；长设备名由
            SelectValue 的 line-clamp-1 截断。 */
          <FilterSelect
            ariaLabel={t("library.scope.all")}
            allLabel={t("library.scope.all")}
            options={deviceOptions.map((o) => ({
              value: o.id,
              label: o.label,
            }))}
            value={deviceScope}
            onChange={setDeviceScope}
          />
        ) : null}

        {!atRoot ? (
          /* Breadcrumb shares the back-button row — flex-1 pushes the
             right-hand group to the row end; each crumb truncates so a deep
             path can't overflow the row. */
          <div className="text-muted-foreground flex min-w-0 flex-1 items-center gap-1 text-xs">
            {breadcrumb.map((c, i) => (
              <span key={c.key} className="flex min-w-0 items-center gap-1">
                {i > 0 ? <ChevronRight className="size-3 shrink-0" /> : null}
                <button
                  type="button"
                  className="hover:text-foreground min-w-0 truncate"
                  onClick={c.onClick}
                >
                  {c.label}
                </button>
              </span>
            ))}
          </div>
        ) : null}

        <div className="ml-auto flex shrink-0 items-center gap-2">
          {/* Search filters the current directory client-side — the scan
              returns a whole directory, so no backend round-trip. Same shape
              as the sessions toolbar search. */}
          <div className="relative w-56">
            <Search className="text-muted-foreground absolute top-1/2 left-2 size-3.5 -translate-y-1/2" />
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("library.searchPlaceholder")}
              aria-label={t("library.searchAria")}
              className="h-8 pl-7"
            />
          </div>

          <Button size="sm" onClick={onAddFiles}>
            <FilePlus />
            {t("library.add")}
          </Button>
        </div>
      </div>

      <Card
        className={cn(
          "flex min-h-0 flex-1 flex-col transition-colors",
          dragging && "border-accent-brand bg-accent-tint",
        )}
      >
        <CardHeader>
          <CardTitle>{t("library.title")}</CardTitle>
        </CardHeader>
        <CardContent className="flex min-h-0 flex-1 flex-col">
          {/* 加载/错误态统一走 QueryState（与 sessions/pricing/logs 同一呈现：
              居中 Skeleton / 可重试错误）。扫描失败不再伪装成「空文件夹」
              空态——错误与真空目录是两种状态两种出路。空态/表格作为 children：
              未 loading、未 error 时由 QueryState 原样渲染。 */}
          <QueryState
            isLoading={isLoading}
            error={scanError}
            isEmpty={false}
            errorAction={{
              label: t("common.retry"),
              onClick: () => void refetchScan(),
            }}
          >
            {totalCount === 0 ? (
              search.trim() ? (
                /* Searched but nothing matched — a lighter state than the
                 empty-folder invite, so "no matches" ≠ "add files". */
                <div className="text-muted-foreground flex flex-1 items-center justify-center py-12 text-sm">
                  {t("library.noMatch")}
                </div>
              ) : (
                <div className="flex flex-1 items-center justify-center">
                  <EmptyState
                    icon={Folder}
                    title={t("library.empty.title")}
                    description={t("library.empty.desc")}
                  />
                </div>
              )
            ) : (
              <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
                {/* table-fixed: column widths come from the header row, so the
                  name column never changes width when a row switches to the
                  inline rename editor (under auto layout the input's intrinsic
                  width stretched the column and shoved the other columns
                  sideways). The name column takes the remaining space and
                  truncates long names; every other column is fixed-width. */}
                <Table className="table-fixed">
                  <TableHeader>
                    <TableRow>
                      <TableHead className="min-w-32">
                        {t("library.col.name")}
                      </TableHead>
                      <TableHead className="w-24">
                        {t("library.col.kind")}
                      </TableHead>
                      <TableHead className="w-24">
                        {t("library.col.size")}
                      </TableHead>
                      <TableHead className="w-40">
                        {showDevice
                          ? t("library.col.device")
                          : t("library.col.modified")}
                      </TableHead>
                      <TableHead className="w-32 text-right">
                        {t("library.col.actions")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {entries.map((e) => {
                      const Icon = kindIcon(e.name, e.kind === "dir")
                      const isRenaming = renaming === e.rel_path
                      const busy = busyRelPath === e.rel_path
                      const kindLabel =
                        e.kind === "dir"
                          ? t("library.kind.dir")
                          : e.name.split(".").pop()?.toUpperCase() ||
                            t("library.kind.file")
                      const deviceLabel = deviceOptionLabel(
                        { is_self: e.is_self, display_name: e.device_name },
                        t,
                      )
                      return (
                        <TableRow
                          key={e.rel_path}
                          // 整行可点：目录钻入 / 文件预览（与名称按钮同一动
                          // 作）。行内挂着重命名编辑器时整行不触发——点击输
                          // 入框不该误开预览/钻入。
                          className={cn(!isRenaming && "cursor-pointer")}
                          onClick={() => {
                            if (isRenaming) return
                            if (e.kind === "dir") drill(e)
                            else setPreview(e)
                          }}
                          onKeyDown={(ev) => {
                            // target 守卫：行内按钮/输入框的 Enter 不带出行动作
                            if (isRenaming || ev.target !== ev.currentTarget)
                              return
                            if (ev.key === "Enter" || ev.key === " ") {
                              ev.preventDefault()
                              if (e.kind === "dir") drill(e)
                              else setPreview(e)
                            }
                          }}
                          tabIndex={isRenaming ? undefined : 0}
                        >
                          <TableCell>
                            {isRenaming ? (
                              /* w-full tracks the fixed table-fixed column, so
                               the editor can't widen the name column (under
                               auto layout the input's intrinsic width would
                               stretch it and shift every row sideways).
                               Confirm/cancel float inside the input so they
                               take no layout width.
                               Blur = commit: clicking away saves instead of
                               leaving a stray editor open. The two inner
                               buttons preventDefault on mousedown so the blur
                               never fires for them — the confirm button would
                               otherwise double-submit (blur commit + click
                               commit) and the cancel button would commit
                               before canceling. */
                              <div className="relative w-full">
                                <Input
                                  value={renameVal}
                                  onChange={(ev) =>
                                    setRenameVal(ev.target.value)
                                  }
                                  className="h-7 w-full pr-16"
                                  onKeyDown={(ev) => {
                                    if (ev.key === "Enter") commitRename(e)
                                    if (ev.key === "Escape") cancelRename()
                                  }}
                                  onBlur={() => commitRename(e)}
                                  autoFocus
                                />
                                <div className="absolute top-1/2 right-1 flex -translate-y-1/2 gap-0.5">
                                  <Button
                                    variant="ghost"
                                    size="icon-sm"
                                    disabled={busy}
                                    onMouseDown={(ev) => ev.preventDefault()}
                                    onClick={() => commitRename(e)}
                                  >
                                    {busy ? (
                                      <Loader2 className="animate-spin" />
                                    ) : (
                                      <Check />
                                    )}
                                  </Button>
                                  <Button
                                    variant="ghost"
                                    size="icon-sm"
                                    onMouseDown={(ev) => ev.preventDefault()}
                                    onClick={cancelRename}
                                  >
                                    <X />
                                  </Button>
                                </div>
                              </div>
                            ) : (
                              <button
                                type="button"
                                className={cn(
                                  /* min-w-0 max-w-full: under table-fixed the
                                   cell is clipped to the column — the flex
                                   button must shrink and let the span
                                   truncate instead of spilling into the next
                                   column. */
                                  "hover:text-accent-brand-strong flex min-w-0 max-w-full items-center gap-2",
                                  /* Directories read heavier so the structure
                                   scans at a glance (file-manager norm). */
                                  e.kind === "dir" && "font-medium",
                                )}
                                onClick={(ev) => {
                                  // 行已是触发器；按钮截停冒泡，避免
                                  // drill/preview 被行处理器再触发一次
                                  // （drill 两次会压两层导航历史）。
                                  ev.stopPropagation()
                                  if (e.kind === "dir") drill(e)
                                  else setPreview(e)
                                }}
                              >
                                <Icon className="size-4 shrink-0" />
                                <Tooltip>
                                  <TooltipTrigger
                                    render={
                                      <span className="min-w-0 truncate">
                                        {e.name}
                                      </span>
                                    }
                                  />
                                  <TooltipContent>{e.name}</TooltipContent>
                                </Tooltip>
                              </button>
                            )}
                          </TableCell>
                          <TableCell className="text-muted-foreground text-xs">
                            {kindLabel}
                          </TableCell>
                          <TableCell className="text-muted-foreground text-xs tabular-nums">
                            {formatSize(e.size)}
                          </TableCell>
                          {/* Tooltip keeps the clipped device name readable —
                            the column is fixed at w-40 under table-fixed. */}
                          <TableCell className="text-muted-foreground text-xs truncate">
                            {showDevice ? (
                              e.is_self ? (
                                deviceLabel
                              ) : (
                                <Tooltip>
                                  <TooltipTrigger
                                    render={<span>{deviceLabel}</span>}
                                  />
                                  <TooltipContent>{deviceLabel}</TooltipContent>
                                </Tooltip>
                              )
                            ) : (
                              <Tooltip>
                                <TooltipTrigger
                                  render={
                                    <span>
                                      {dayjs(e.modified_ms).fromNow()}
                                    </span>
                                  }
                                />
                                <TooltipContent>
                                  {dayjs(e.modified_ms).format("MM/DD HH:mm")}
                                </TooltipContent>
                              </Tooltip>
                            )}
                          </TableCell>
                          <TableCell className="text-right">
                            <div className="flex justify-end gap-1">
                              <Tooltip>
                                <TooltipTrigger
                                  render={
                                    <Button
                                      variant="ghost"
                                      size="icon-sm"
                                      aria-label={t("library.row.export")}
                                      disabled={busy}
                                      onClick={(ev) => {
                                        ev.stopPropagation()
                                        onExport(e)
                                      }}
                                    />
                                  }
                                >
                                  {busy ? (
                                    <Loader2 className="animate-spin" />
                                  ) : (
                                    <Download />
                                  )}
                                </TooltipTrigger>
                                <TooltipContent>
                                  {t("library.row.export")}
                                </TooltipContent>
                              </Tooltip>
                              <Tooltip>
                                <TooltipTrigger
                                  render={
                                    <Button
                                      variant="ghost"
                                      size="icon-sm"
                                      aria-label={t("library.row.rename")}
                                      disabled={busy}
                                      onClick={(ev) => {
                                        ev.stopPropagation()
                                        startRename(e)
                                      }}
                                    />
                                  }
                                >
                                  <Pencil />
                                </TooltipTrigger>
                                <TooltipContent>
                                  {t("library.row.rename")}
                                </TooltipContent>
                              </Tooltip>
                              <Tooltip>
                                <TooltipTrigger
                                  render={
                                    <Button
                                      variant="ghost"
                                      size="icon-sm"
                                      aria-label={t("library.row.delete")}
                                      disabled={busy}
                                      onClick={(ev) => {
                                        ev.stopPropagation()
                                        confirmDelete.requestDelete(e)
                                      }}
                                    />
                                  }
                                >
                                  {busy ? (
                                    <Loader2 className="animate-spin" />
                                  ) : (
                                    <Trash2 />
                                  )}
                                </TooltipTrigger>
                                <TooltipContent>
                                  {t("library.row.delete")}
                                </TooltipContent>
                              </Tooltip>
                            </div>
                          </TableCell>
                        </TableRow>
                      )
                    })}
                  </TableBody>
                </Table>
              </div>
            )}
          </QueryState>

          {/* Paged footer — the shared PaginationBar (page info left, numbered
            pages with ellipsis jumps right). */}
          <PaginationBar
            page={page}
            totalPages={totalPages}
            total={totalCount}
            onPageChange={goToPage}
          />
        </CardContent>
      </Card>

      {dragging ? (
        <div className="pointer-events-none fixed inset-0 z-30 flex items-center justify-center">
          <div className="border-accent-brand bg-accent-tint text-accent-brand-strong rounded-xl border-2 border-dashed px-8 py-6 text-sm font-medium">
            {/* Name the drop target so a stray drop lands somewhere visible,
                not the directory you happened to be browsing. */}
            {subpath
              ? t("library.drop.target", { path: subpath })
              : t("library.drop.active")}
          </div>
        </div>
      ) : null}

      {pendingPaths ? (
        <UploadDialog
          paths={pendingPaths}
          subpath={subpath}
          onClose={clearPendingPaths}
        />
      ) : null}

      {preview ? (
        <PreviewSheet
          entry={preview}
          busy={busyRelPath === preview.rel_path}
          onClose={() => setPreview(null)}
          onExport={() => onExport(preview)}
          /* Rename hands back to the row's inline editor (one rename UI, not
             a second copy inside the sheet). */
          onRename={() => {
            setPreview(null)
            startRename(preview)
          }}
        />
      ) : null}

      <ConfirmDialog
        open={confirmDelete.deleting !== null}
        onOpenChange={(open) => {
          if (!open) confirmDelete.cancel()
        }}
        title={t("confirm.deleteTitle", {
          name: confirmDelete.deleting?.name ?? "",
        })}
        description={t("library.confirm.deleteDesc", {
          name: confirmDelete.deleting?.name ?? "",
        })}
        confirmLabel={t("common.delete")}
        onConfirm={() => void confirmDelete.onConfirmDelete()}
      />
    </div>
  )
}
