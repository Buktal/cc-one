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
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState } from "@/components/empty-state"
import { PaginationBar } from "@/components/pagination-bar"
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { formatSize } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { LibraryEntry } from "@/types/generated/bindings"
import { kindIcon } from "../kind-icon"
import {
  ALL,
  LIBRARY_PAGE_SIZE,
  useLibraryBrowser,
} from "../use-library-browser"
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
    setOffset,
    isLoading,
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
    onDelete,
    startRename,
    commitRename,
    preview,
    setPreview,
  } = useLibraryBrowser()

  // 待删除条目（非 null 弹确认框）。先关再删：行内已有 busyRelPath spinner，
  // 确认后立刻关框、由行级 busy 接管（无需 busy 态）。
  const [deleting, setDeleting] = useState<LibraryEntry | null>(null)
  function onConfirmDelete() {
    const entry = deleting
    setDeleting(null)
    if (entry) void onDelete(entry)
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
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
          <Select
            value={deviceScope}
            onValueChange={(v) => setDeviceScope(v ?? ALL)}
          >
            <SelectTrigger
              className="border-border bg-card hover:bg-muted/60 h-8 w-40 rounded-md"
              aria-label={t("library.scope.all")}
            >
              <SelectValue className="min-w-0">
                {(value: string) =>
                  value === ALL
                    ? t("library.scope.all")
                    : (deviceOptions.find((o) => o.id === value)?.label ??
                      value)
                }
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={ALL}>{t("library.scope.all")}</SelectItem>
              {deviceOptions.map((o) => (
                <SelectItem key={o.id} value={o.id}>
                  {o.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        ) : null}

        {!atRoot ? (
          /* Breadcrumb shares the back-button row — flex-1 pushes the Add
             button to the right; each crumb truncates so a deep path can't
             overflow the row. */
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
        ) : (
          <div className="ml-auto" />
        )}

        {/* Search filters the current directory client-side — the scan returns
            a whole directory, so no backend round-trip. Same shape as the
            sessions toolbar search. */}
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
          {isLoading ? (
            <div className="text-muted-foreground p-4 text-sm">
              {t("common.loading")}
            </div>
          ) : totalCount === 0 ? (
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
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("library.col.name")}</TableHead>
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
                    <TableHead className="text-right">
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
                    return (
                      <TableRow key={e.rel_path}>
                        <TableCell>
                          {isRenaming ? (
                            /* w-full tracks the name column — the inline
                               editor must not widen the column (a fixed-width
                               input would shift every row sideways while
                               renaming). Confirm/cancel float inside the input
                               so they take no layout width. */
                            <div className="relative w-full">
                              <Input
                                value={renameVal}
                                onChange={(ev) => setRenameVal(ev.target.value)}
                                className="h-7 w-full pr-16"
                                onKeyDown={(ev) => {
                                  if (ev.key === "Enter") commitRename(e)
                                  if (ev.key === "Escape") cancelRename()
                                }}
                                autoFocus
                              />
                              <div className="absolute top-1/2 right-1 flex -translate-y-1/2 gap-0.5">
                                <Button
                                  variant="ghost"
                                  size="icon-sm"
                                  disabled={busy}
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
                                "hover:text-accent-brand-strong flex items-center gap-2",
                                /* Directories read heavier so the structure
                                   scans at a glance (file-manager norm). */
                                e.kind === "dir" && "font-medium",
                              )}
                              onClick={() =>
                                e.kind === "dir" ? drill(e) : setPreview(e)
                              }
                            >
                              <Icon className="size-4 shrink-0" />
                              <span>{e.name}</span>
                            </button>
                          )}
                        </TableCell>
                        <TableCell className="text-muted-foreground text-xs">
                          {kindLabel}
                        </TableCell>
                        <TableCell className="text-muted-foreground text-xs tabular-nums">
                          {formatSize(e.size)}
                        </TableCell>
                        <TableCell className="text-muted-foreground text-xs">
                          {showDevice ? (
                            e.is_self ? (
                              t("devices.thisDevice")
                            ) : (
                              e.device_name
                            )
                          ) : (
                            <span
                              title={dayjs(e.modified_ms).format("MM/DD HH:mm")}
                            >
                              {dayjs(e.modified_ms).fromNow()}
                            </span>
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
                                    onClick={() => onExport(e)}
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
                                    onClick={() => startRename(e)}
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
                                    onClick={() => setDeleting(e)}
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

          {/* Paged footer — the shared PaginationBar (page info left, numbered
            pages with ellipsis jumps right; disabled states agree with the
            slice size via LIBRARY_PAGE_SIZE). */}
          <PaginationBar
            page={page}
            totalPages={totalPages}
            total={totalCount}
            onPageChange={(p) => setOffset((p - 1) * LIBRARY_PAGE_SIZE)}
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
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null)
        }}
        title={t("confirm.deleteTitle", { name: deleting?.name ?? "" })}
        description={t("library.confirm.deleteDesc", {
          name: deleting?.name ?? "",
        })}
        confirmLabel={t("common.delete")}
        onConfirm={onConfirmDelete}
      />
    </div>
  )
}
