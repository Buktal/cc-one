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
  File as FileIcon,
  FileJson,
  FilePlus,
  FileText,
  Folder,
  Image as ImageIcon,
  Loader2,
  Pencil,
  Trash2,
  X,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { EmptyState } from "@/components/empty-state"
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
import { formatInt, formatSize } from "@/lib/format"
import { cn } from "@/lib/utils"
import {
  ALL,
  LIBRARY_PAGE_SIZE,
  useLibraryBrowser,
} from "../use-library-browser"
import { PreviewSheet } from "./preview-sheet"
import { UploadDialog } from "./upload-dialog"

dayjs.extend(relativeTime)

function kindIcon(name: string, isDir: boolean) {
  if (isDir) return Folder
  const ext = name.split(".").pop()?.toLowerCase()
  if (!ext) return FileIcon
  if (ext === "json") return FileJson
  if (["md", "markdown", "txt", "log"].includes(ext)) return FileText
  if (["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"].includes(ext))
    return ImageIcon
  return FileIcon
}

export function LibraryView() {
  const { t } = useTranslation()
  const {
    entries,
    totalCount,
    page,
    totalPages,
    offset,
    setOffset,
    isLoading,
    deviceOptions,
    deviceScope,
    setDeviceScope,
    subpath,
    atRoot,
    showDevice,
    breadcrumb,
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
          ) : entries.length === 0 ? (
            <div className="flex flex-1 items-center justify-center">
              <EmptyState
                icon={Folder}
                title={t("library.empty.title")}
                description={t("library.empty.desc")}
              />
            </div>
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
                            <div className="flex items-center gap-1">
                              <Input
                                value={renameVal}
                                onChange={(ev) => setRenameVal(ev.target.value)}
                                className="h-7 w-44"
                                onKeyDown={(ev) => {
                                  if (ev.key === "Enter") commitRename(e)
                                  if (ev.key === "Escape") cancelRename()
                                }}
                                autoFocus
                              />
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
                          ) : (
                            <button
                              type="button"
                              className="hover:text-accent-brand-strong flex items-center gap-2"
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
                                    onClick={() => onDelete(e)}
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

          {/* Paged footer — same control as the request-log / sessions tables
            (page info left, prev/next right; disabled states agree with the
            slice size via LIBRARY_PAGE_SIZE). */}
          <div className="text-muted-foreground mt-3 flex shrink-0 items-center justify-between text-xs">
            <span>
              {t("library.pageInfo", {
                page,
                totalPages,
                total: formatInt(totalCount),
              })}
            </span>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={offset === 0}
                onClick={() =>
                  setOffset(Math.max(0, offset - LIBRARY_PAGE_SIZE))
                }
              >
                {t("library.prevPage")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={offset + LIBRARY_PAGE_SIZE >= totalCount}
                onClick={() => setOffset(offset + LIBRARY_PAGE_SIZE)}
              >
                {t("library.nextPage")}
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {dragging ? (
        <div className="pointer-events-none fixed inset-0 z-30 flex items-center justify-center">
          <div className="border-accent-brand bg-accent-tint text-accent-brand-strong rounded-xl border-2 border-dashed px-8 py-6 text-sm font-medium">
            {t("library.drop.active")}
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
        <PreviewSheet entry={preview} onClose={() => setPreview(null)} />
      ) : null}
    </div>
  )
}
