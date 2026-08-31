// Library view — per-device, git-mediated cloud storage. Drag files / dirs in
// to upload (= push to the sync repo); drill into directories (the same surface
// at every depth — drag-in, export, single-file download all work inside);
// preview a file in the webview; export to a path you choose. cc one never
// writes into an AI tool's own config dir.
//
// Presentation + the inline rename editor's wiring: browser state comes from
// useLibraryBrowser as clusters (query / nav / upload / actions / preview),
// the rename editor's draft/open/busy machine is useInlineEdit paired with
// InlineTextEdit (the Enter / Escape / blur / button contract lives there).
// This component owns JSX, styles, i18n, and the pure display helper
// (kindIcon).

import {
  ArrowUp,
  ChevronRight,
  Download,
  FilePlus,
  Folder,
  Loader2,
  Pencil,
  Search,
  Trash2,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { collapseTriggerProps } from "@/components/collapse-trigger"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState } from "@/components/empty-state"
import { FilterSelect } from "@/components/filter-select"
import { InlineTextEdit } from "@/components/inline-text-edit"
import { PaginationBar } from "@/components/pagination-bar"
import { QueryState } from "@/components/query-state"
import { RelativeTime } from "@/components/relative-time"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
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
import { useConfirmAction } from "@/hooks/use-confirm-action"
import { useInlineEdit } from "@/hooks/use-inline-edit"
import { deviceOptionLabel } from "@/lib/device-labels"
import { formatSize } from "@/lib/format"
import { cn } from "@/lib/utils"
import type { LibraryEntry } from "@/types/generated/bindings"
import { extOf } from "../derive"
import { kindIcon } from "../kind-icon"
import { useLibraryBrowser } from "../use-library-browser"
import { PreviewSheet } from "./preview-sheet"
import { UploadDialog } from "./upload-dialog"

export function LibraryView() {
  const { t } = useTranslation()
  const { query, nav, upload, actions, preview } = useLibraryBrowser()
  // 预览的 entry 拆成局部 const：JSX 守卫的收窄能带进回调（属性访问
  // preview.entry 带不进去）。
  const { entry: previewed, open: openPreview, close: closePreview } = preview
  // 行内重命名编辑器：draft/open/busy 机器归 useInlineEdit，业务动作归
  // browser.actions.rename。target 存 begin 时抓的 entry，按 rel_path 比对
  // ——扫描重取换了对象引用也不丢编辑态。
  const rename = useInlineEdit<LibraryEntry>({ commit: actions.rename })

  // 删除确认（busy / 关闭时序收敛在 useConfirmAction）。先关再删：行内已有
  // busyRelPath spinner，closeFirst 模式确认后立刻关框、由行级 busy 接管。
  const confirmDelete = useConfirmAction<LibraryEntry>({
    holdOpen: false,
    onAction: async (entry) => {
      await actions.remove(entry)
      return true
    },
  })

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      {/* Toolbar in a fixed element order: navigation (up / device picker /
        breadcrumb) on the left, search + add pinned right as one group —
        on a narrow container the group drops to its own right-aligned line
        instead of scattering between the breadcrumb crumbs. */}
      <div className="@container flex shrink-0 flex-wrap items-center gap-2">
        {!nav.atRoot ? (
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  variant="outline"
                  size="icon-sm"
                  aria-label={t("library.up")}
                  onClick={nav.goUp}
                />
              }
            >
              <ArrowUp />
            </TooltipTrigger>
            <TooltipContent>{t("library.up")}</TooltipContent>
          </Tooltip>
        ) : null}

        {nav.atRoot ? (
          /* 宽度策略（自适应 + 上限）收敛在 FilterSelect 本体；长设备名由
            SelectValue 的 line-clamp-1 截断。 */
          <FilterSelect
            ariaLabel={t("library.scope.all")}
            allLabel={t("library.scope.all")}
            options={nav.deviceOptions.map((o) => ({
              value: o.id,
              label: o.label,
            }))}
            value={nav.deviceScope}
            onChange={nav.pickDevice}
          />
        ) : null}

        {!nav.atRoot ? (
          /* Breadcrumb shares the back-button row — flex-1 pushes the
             right-hand group to the row end; each crumb truncates so a deep
             path can't overflow the row. */
          <div className="text-muted-foreground flex min-w-0 flex-1 items-center gap-1 text-xs">
            {nav.breadcrumb.map((c, i) => (
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
              value={query.search}
              onChange={(e) => query.onSearch(e.target.value)}
              placeholder={t("library.searchPlaceholder")}
              aria-label={t("library.searchAria")}
              className="h-8 pl-7"
            />
          </div>

          <Button size="sm" onClick={upload.addFiles}>
            <FilePlus />
            {t("library.add")}
          </Button>
        </div>
      </div>

      <Card
        className={cn(
          "flex min-h-0 flex-1 flex-col transition-colors",
          upload.dragging && "border-accent-brand bg-accent-tint",
        )}
      >
        <CardContent className="flex min-h-0 flex-1 flex-col">
          {/* 加载/错误态统一走 QueryState（与 sessions/pricing/logs 同一呈现：
              居中 Skeleton / 可重试错误）。扫描失败不再伪装成「空文件夹」
              空态——错误与真空目录是两种状态两种出路。空态/表格作为 children：
              未 loading、未 error 时由 QueryState 原样渲染。 */}
          <QueryState
            isLoading={query.isLoading}
            error={query.scanError}
            isEmpty={false}
            errorAction={{
              label: t("common.retry"),
              onClick: () => void query.refetchScan(),
            }}
          >
            {query.totalCount === 0 ? (
              query.search.trim() ? (
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
                        {nav.showDevice
                          ? t("library.col.device")
                          : t("library.col.modified")}
                      </TableHead>
                      <TableHead className="w-32 text-right">
                        {t("library.col.actions")}
                      </TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {query.entries.map((e) => {
                      const Icon = kindIcon(e.name, e.kind === "dir")
                      const isRenaming = rename.target?.rel_path === e.rel_path
                      const busy = actions.busyRelPath === e.rel_path
                      const kindLabel =
                        e.kind === "dir"
                          ? t("library.kind.dir")
                          : extOf(e.name).toUpperCase() ||
                            t("library.kind.file")
                      const deviceLabel = deviceOptionLabel(
                        { is_self: e.is_self, display_name: e.device_name },
                        t,
                      )
                      return (
                        <TableRow
                          key={e.rel_path}
                          // 整行可点：目录钻入 / 文件预览（与名称按钮同一动
                          // 作）。行内挂着重命名编辑器时整份触发契约卸下
                          // （enabled=false：无焦点位、点击键盘都不响应）——
                          // 点击输入框不该误开预览/钻入。target 守卫
                          // （selfTargetOnly）让行内按钮/输入框的 Enter 不带
                          // 出行动作。契约由 collapseTriggerProps 工厂给出。
                          {...collapseTriggerProps({
                            onToggle: () => {
                              if (e.kind === "dir") nav.drill(e)
                              else openPreview(e)
                            },
                            selfTargetOnly: true,
                            enabled: !isRenaming,
                          })}
                          className={cn(!isRenaming && "cursor-pointer")}
                        >
                          <TableCell>
                            {isRenaming ? (
                              /* 编辑器收进 InlineTextEdit，w-full 跟住
                                 table-fixed 的列宽（编辑态不撑宽名字列）；
                                 Enter 提交 / Escape 取消 / 失焦提交（空草稿
                                 视为放弃）、✓/✕ mousedown preventDefault
                                 不夺焦点——契约单一归属（见组件文件头）。 */
                              <InlineTextEdit
                                className="w-full"
                                value={rename.draft}
                                onValueChange={rename.setDraft}
                                busy={busy}
                                onCommit={() => void rename.commit()}
                                onCancel={rename.cancel}
                                autoFocus
                              />
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
                                  if (e.kind === "dir") nav.drill(e)
                                  else openPreview(e)
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
                            {nav.showDevice ? (
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
                              <RelativeTime ts={e.modified_ms} />
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
                                        void actions.export(e)
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
                                        rename.begin(e, e.name)
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
                                        confirmDelete.request(e)
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
            pages with ellipsis jumps + per-page density right). */}
          <PaginationBar
            page={query.page}
            totalPages={query.totalPages}
            total={query.totalCount}
            onPageChange={query.goToPage}
            density={query.density}
          />
        </CardContent>
      </Card>

      {upload.dragging ? (
        <div className="pointer-events-none fixed inset-0 z-30 flex items-center justify-center">
          <div className="border-accent-brand bg-accent-tint text-accent-brand-strong rounded-xl border-2 border-dashed px-8 py-6 text-sm font-medium">
            {/* Name the drop target so a stray drop lands somewhere visible,
                not the directory you happened to be browsing. */}
            {nav.subpath
              ? t("library.drop.target", { path: nav.subpath })
              : t("library.drop.active")}
          </div>
        </div>
      ) : null}

      {upload.pendingPaths ? (
        <UploadDialog
          paths={upload.pendingPaths}
          subpath={nav.subpath}
          onClose={upload.close}
        />
      ) : null}

      {previewed ? (
        <PreviewSheet
          entry={previewed}
          busy={actions.busyRelPath === previewed.rel_path}
          onClose={closePreview}
          onExport={() => void actions.export(previewed)}
          /* Rename hands back to the row's inline editor (one rename UI, not
             a second copy inside the sheet). */
          onRename={() => {
            closePreview()
            rename.begin(previewed, previewed.name)
          }}
        />
      ) : null}

      <ConfirmDialog
        open={confirmDelete.pending !== null}
        onOpenChange={(open) => {
          if (!open) confirmDelete.cancel()
        }}
        title={t("confirm.deleteTitle", {
          name: confirmDelete.pending?.name ?? "",
        })}
        description={t("library.confirm.deleteDesc", {
          name: confirmDelete.pending?.name ?? "",
        })}
        confirmLabel={t("common.delete")}
        onConfirm={() => void confirmDelete.confirm()}
      />
    </div>
  )
}
