// Pricing view (BLUEPRINT 成本定价): model pricing table with add /
// edit (via Dialog) / delete (via ConfirmDialog), LiteLLM upstream fetch as the
// primary text-button action, pricing.json import/export as icon buttons, plus
// client-side search and single-column sort. Price columns keep the `$/1M`
// unit in the header so cells stay bare numbers. Empty state routes a fresh
// install to 拉取 LiteLLM; a search miss gets a lightweight in-table row. Table
// state (data, search/sort/pagination, delete) lives in usePricingTable; this
// file is just the render — dialog UI and the fetch/reload/save toolbar
// actions.

import {
  ChevronUp,
  CloudDownload,
  FileDown,
  FileUp,
  Pencil,
  Plus,
  Search,
  Trash2,
} from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useFetchLitellmMutation,
  useReloadPricingMutation,
  useSavePricingToFileMutation,
} from "@/app/store/api"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { PaginationBar } from "@/components/pagination-bar"
import { QueryState } from "@/components/query-state"
import { Badge } from "@/components/ui/badge"
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
import type { PricingSortKey } from "@/features/pricing/derive"
import { usePricingTable } from "@/features/pricing/use-pricing-table"
import { useConfirmAction } from "@/hooks/use-confirm-action"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { formatCostAmount } from "@/lib/format"

import type { PricingEntry } from "@/types/generated/bindings"
import { EntryEditorDialog, emptyEntry } from "./entry-editor-dialog"

export function PricingView() {
  const { t } = useTranslation()
  const [fetchLitellm, { isLoading: fetching }] = useFetchLitellmMutation()
  const [reloadFile, { isLoading: reloading }] = useReloadPricingMutation()
  const [saveFile, { isLoading: savingFile }] = useSavePricingToFileMutation()
  const runWithToast = useMutateWithToast()

  const {
    isLoading,
    error,
    remove,
    search,
    setSearch,
    sortKey,
    sortDir,
    onSort,
    total,
    page,
    totalPages,
    paged,
    goToPage,
    density,
  } = usePricingTable()

  const [editing, setEditing] = useState<PricingEntry | null>(null)
  const [dialogOpen, setDialogOpen] = useState(false)
  // 删除确认（busy / 关闭时序收敛在 useConfirmAction，holdOpen：成功才关框）。
  const confirmDelete = useConfirmAction<PricingEntry>({
    onAction: (entry) => remove(entry.model_key),
  })

  function openNew() {
    setEditing(emptyEntry())
    setDialogOpen(true)
  }
  function openEdit(e: PricingEntry) {
    setEditing({ ...e })
    setDialogOpen(true)
  }

  async function onFetchLitellm() {
    await runWithToast(fetchLitellm, undefined, {
      success: {
        message: (count) => t("pricing.toast.fetched", { count: count ?? 0 }),
      },
      failed: { key: "pricing.toast.fetchFailed" },
    })
  }
  async function onReloadFile() {
    await runWithToast(reloadFile, undefined, {
      success: {
        message: (count) => t("pricing.toast.reloaded", { count: count ?? 0 }),
      },
      failed: { key: "pricing.toast.reloadFailed" },
    })
  }
  async function onSaveFile() {
    await runWithToast(saveFile, undefined, {
      success: { key: "pricing.toast.savedFile" },
      failed: { key: "pricing.toast.saveFileFailed" },
    })
  }

  const sortProps = { sortKey, sortDir, onSort }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      {/* Toolbar in a fixed element order: search + fetch + import/export
        form one group that wraps as a unit; the primary action (新增) pins
        right via ml-auto and drops to its own right-aligned line on narrow
        containers — the old spacer div was a no-op once the row wrapped.
        shrink-0：满高布局下工具行是卡片上方的兄弟 flex 项，不被压缩。 */}
      <div className="@container flex shrink-0 flex-wrap items-center gap-2">
        <div className="flex shrink-0 flex-wrap items-center gap-2">
          <div className="relative">
            <Search className="text-muted-foreground absolute top-1/2 left-2 size-3.5 -translate-y-1/2" />
            <Input
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder={t("pricing.searchPlaceholder")}
              className="h-8 w-44 pl-7"
              aria-label={t("pricing.searchAria")}
            />
          </div>
          {/* 拉取 LiteLLM 是种子数据的来源——主动作带文字，导入/导出留在 icon
            组里保持密度。 */}
          <Button
            variant="outline"
            size="sm"
            disabled={fetching}
            onClick={onFetchLitellm}
          >
            <CloudDownload />
            {t("pricing.fetchLitellm")}
          </Button>
          <div className="flex items-center gap-1">
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="outline"
                    size="icon-sm"
                    disabled={reloading}
                    onClick={onReloadFile}
                    aria-label={t("pricing.importFile")}
                  />
                }
              >
                <FileUp />
              </TooltipTrigger>
              <TooltipContent>{t("pricing.importFile")}</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="outline"
                    size="icon-sm"
                    disabled={savingFile}
                    onClick={onSaveFile}
                    aria-label={t("pricing.exportFile")}
                  />
                }
              >
                <FileDown />
              </TooltipTrigger>
              <TooltipContent>{t("pricing.exportFile")}</TooltipContent>
            </Tooltip>
          </div>
        </div>
        <div className="ml-auto">
          <Button size="sm" onClick={openNew}>
            <Plus />
            {t("pricing.add")}
          </Button>
        </div>
      </div>
      <Card className="min-h-0 flex-1">
        <CardContent className="flex min-h-0 flex-1 flex-col">
          {/* 空态两分支：数据源为空 → EmptyState 引导拉取或新增；搜索无结果 →
            表格内的轻量提示行（total === 0 且搜索非空）。 */}
          <QueryState
            isLoading={isLoading}
            error={error}
            isEmpty={!isLoading && total === 0 && !search.trim()}
            emptyIcon={CloudDownload}
            emptyLabel={t("pricing.empty.title")}
            emptyDescription={t("pricing.empty.desc")}
            emptyAction={{
              label: t("pricing.fetchLitellm"),
              onClick: onFetchLitellm,
              disabled: fetching,
            }}
          >
            <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>
                      <SortHeader
                        label={t("pricing.col.modelKey")}
                        k="model_key"
                        {...sortProps}
                      />
                    </TableHead>
                    <TableHead>
                      <SortHeader
                        label={t("pricing.col.displayName")}
                        k="display_name"
                        {...sortProps}
                      />
                    </TableHead>
                    <TableHead>
                      <SortHeader
                        label={t("usage.tokens.input")}
                        k="input_per_million"
                        unit="$/1M"
                        {...sortProps}
                      />
                    </TableHead>
                    <TableHead>
                      <SortHeader
                        label={t("usage.tokens.output")}
                        k="output_per_million"
                        unit="$/1M"
                        {...sortProps}
                      />
                    </TableHead>
                    <TableHead>
                      <SortHeader
                        label={t("usage.tokens.cacheRead")}
                        k="cache_read_per_million"
                        unit="$/1M"
                        {...sortProps}
                      />
                    </TableHead>
                    <TableHead>
                      <SortHeader
                        label={t("usage.tokens.cacheCreation")}
                        k="cache_creation_per_million"
                        unit="$/1M"
                        {...sortProps}
                      />
                    </TableHead>
                    <TableHead>{t("usage.logs.col.source")}</TableHead>
                    <TableHead className="text-right">
                      {t("pricing.col.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {total === 0 ? (
                    <TableRow>
                      <TableCell
                        colSpan={8}
                        className="text-muted-foreground py-8 text-center"
                      >
                        {t("pricing.noMatch")}
                      </TableCell>
                    </TableRow>
                  ) : (
                    paged.map((e) => (
                      <TableRow key={e.model_key}>
                        <TableCell className="font-mono text-xs">
                          {e.model_key}
                        </TableCell>
                        <TableCell>{e.display_name}</TableCell>
                        <TableCell className="pr-4 text-right tabular-nums">
                          {formatCostAmount(e.input_per_million)}
                        </TableCell>
                        <TableCell className="pr-4 text-right tabular-nums">
                          {formatCostAmount(e.output_per_million)}
                        </TableCell>
                        <TableCell className="pr-4 text-right tabular-nums">
                          {formatCostAmount(e.cache_read_per_million)}
                        </TableCell>
                        <TableCell className="pr-4 text-right tabular-nums">
                          {formatCostAmount(e.cache_creation_per_million)}
                        </TableCell>
                        <TableCell>
                          <Badge
                            variant={e.is_builtin ? "secondary" : "default"}
                          >
                            {e.is_builtin
                              ? t("pricing.builtin")
                              : t("pricing.custom")}
                          </Badge>
                        </TableCell>
                        <TableCell className="text-right">
                          <div className="flex justify-end gap-1">
                            <Tooltip>
                              <TooltipTrigger
                                render={
                                  <Button
                                    variant="ghost"
                                    size="icon-sm"
                                    onClick={() => openEdit(e)}
                                    aria-label={t("common.edit")}
                                  />
                                }
                              >
                                <Pencil />
                              </TooltipTrigger>
                              <TooltipContent>
                                {t("common.edit")}
                              </TooltipContent>
                            </Tooltip>
                            <Tooltip>
                              <TooltipTrigger
                                render={
                                  <Button
                                    variant="ghost"
                                    size="icon-sm"
                                    onClick={() => confirmDelete.request(e)}
                                    aria-label={t("common.delete")}
                                  />
                                }
                              >
                                <Trash2 />
                              </TooltipTrigger>
                              <TooltipContent>
                                {t("common.delete")}
                              </TooltipContent>
                            </Tooltip>
                          </div>
                        </TableCell>
                      </TableRow>
                    ))
                  )}
                </TableBody>
              </Table>
            </div>
          </QueryState>

          <PaginationBar
            page={page}
            totalPages={totalPages}
            total={total}
            onPageChange={goToPage}
            density={density}
          />
        </CardContent>
      </Card>

      <EntryEditorDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        entry={editing}
        onSaved={() => {
          setDialogOpen(false)
          setEditing(null)
        }}
      />

      <ConfirmDialog
        open={confirmDelete.pending !== null}
        onOpenChange={(o) => {
          if (!o) confirmDelete.cancel()
        }}
        title={t("confirm.deleteTitle", {
          name: confirmDelete.pending?.model_key ?? "",
        })}
        description={t("pricing.confirm.deleteDesc", {
          name: confirmDelete.pending?.model_key ?? "",
        })}
        confirmLabel={t("common.delete")}
        busy={confirmDelete.busy}
        onConfirm={() => void confirmDelete.confirm()}
      />
    </div>
  )
}

function SortHeader({
  label,
  unit,
  k,
  sortKey,
  sortDir,
  onSort,
}: {
  label: string
  /** 列单位小标（如 `$/1M`）——单元格保持纯数字，单位放表头不重复。 */
  unit?: string
  k: PricingSortKey
  sortKey: PricingSortKey | null
  sortDir: "asc" | "desc"
  onSort: (k: PricingSortKey) => void
}) {
  const active = sortKey === k
  return (
    <button
      type="button"
      onClick={() => onSort(k)}
      className={`inline-flex items-center gap-1 transition-colors hover:text-foreground ${
        active ? "text-foreground" : ""
      }`}
    >
      {label}
      {unit ? (
        <span className="text-muted-foreground text-[10px] font-normal">
          {unit}
        </span>
      ) : null}
      <ChevronUp
        className={`size-3 transition-transform ${
          active && sortDir === "desc" ? "rotate-180" : ""
        } ${active ? "opacity-100" : "opacity-0"}`}
      />
    </button>
  )
}
