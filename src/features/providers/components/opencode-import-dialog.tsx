// 「从 opencode.json 导入」专用对话框（附加模式预览）：打开时先读 opencode.json，
// 列出将导入的每个供应商（名称 / 端点 / 是否含密钥 / 新建或更新），确认后才执行
// 导入，完成后内联展示结果报告（取代原来的直接执行 + toast）。自包含：内部调
// previewOpencodeImport / importProvidersFromLive 两个 mutation，`!open` 直接
// unmount 重置状态（与 cc-switch-import-dialog 同一生命周期）。
//
// 视图状态机：loading → missing（文件不存在，带路径）/ error（红块）/ ready 空态
// （无 provider 段）→ ready 预览（摘要 + 条目列表 + 确认）→ result（成功块 + 列表）。
// 密钥不进预览载荷：条目只有 hasSecret 布尔，密钥值永不跨边界（Rust 侧有防泄漏
// 回归测试锁着）。

import {
  AlertCircle,
  CheckCircle2,
  FileJson,
  Loader2,
  Upload,
} from "lucide-react"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useImportProvidersFromLiveMutation,
  usePreviewOpencodeImportMutation,
} from "@/app/store/api"
import { EmptyState } from "@/components/empty-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { toStructuredError } from "@/lib/error"

import type {
  App,
  OpenCodeImportPreviewEntry,
} from "@/types/generated/bindings"

type Phase =
  | { kind: "loading" }
  | { kind: "missing"; path: string }
  | { kind: "error"; message: string }
  | { kind: "ready"; entries: OpenCodeImportPreviewEntry[] }
  | { kind: "result"; imported: number; entries: OpenCodeImportPreviewEntry[] }

/** mutation 错误 → 人类可读消息（AppError 的 data 优先，与 CC-Switch 导入一致）。 */
function errorMessage(error: unknown): string {
  const structured = toStructuredError(error)
  return structured?.kind === "app"
    ? structured.data
    : (structured?.message ?? String(error))
}

export function OpencodeImportDialog({
  open,
  onOpenChange,
  app,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  app: App
}) {
  const { t } = useTranslation()
  const [preview] = usePreviewOpencodeImportMutation()
  const [importFromLive, { isLoading: importing }] =
    useImportProvidersFromLiveMutation()
  const [phase, setPhase] = useState<Phase>({ kind: "loading" })

  // 打开 → 立即预览（mutation 无缓存，每次打开都是新读盘；`!open` 时组件已
  // unmount，本 effect 只在挂载后跑一次）。preview trigger 引用跨渲染稳定，
  // 加进依赖数组无副作用（RTK Query 保证）。
  useEffect(() => {
    setPhase({ kind: "loading" })
    void (async () => {
      const result = await preview(app)
      if (result.error) {
        setPhase({ kind: "error", message: errorMessage(result.error) })
        return
      }
      if (result.data.kind === "missing") {
        setPhase({ kind: "missing", path: result.data.path })
        return
      }
      setPhase({ kind: "ready", entries: result.data.entries })
    })()
  }, [app, preview])

  async function onConfirm() {
    if (phase.kind !== "ready") return
    const result = await importFromLive(app)
    if (result.error) {
      setPhase({ kind: "error", message: errorMessage(result.error) })
      return
    }
    setPhase({ kind: "result", imported: result.data, entries: phase.entries })
  }

  function close() {
    onOpenChange(false)
  }

  if (!open) return null

  const created =
    phase.kind === "ready" || phase.kind === "result"
      ? phase.entries.filter((e) => e.isNew).length
      : 0
  const updated =
    phase.kind === "ready" || phase.kind === "result"
      ? phase.entries.length - created
      : 0

  return (
    <Dialog open onOpenChange={(o) => !o && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("providers.opencodeImport.title")}</DialogTitle>
          <DialogDescription>
            {t("providers.opencodeImport.hint")}
          </DialogDescription>
        </DialogHeader>

        {phase.kind === "loading" ? (
          <div className="text-muted-foreground flex items-center justify-center gap-2 py-8 text-sm">
            <Loader2 className="animate-spin" />
            {t("common.loading")}
          </div>
        ) : phase.kind === "missing" ? (
          <p className="bg-destructive/10 text-destructive flex items-start gap-1.5 rounded-md px-3 py-2 text-xs">
            <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
            <span>
              {t("providers.opencodeImport.missing", { path: phase.path })}
            </span>
          </p>
        ) : phase.kind === "error" ? (
          <p className="bg-destructive/10 text-destructive flex items-start gap-1.5 rounded-md px-3 py-2 text-xs">
            <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
            <span>{phase.message}</span>
          </p>
        ) : phase.kind === "ready" && phase.entries.length === 0 ? (
          <EmptyState
            icon={FileJson}
            title={t("providers.opencodeImport.empty")}
          />
        ) : phase.kind === "ready" ? (
          <div className="flex flex-col gap-3">
            <p className="text-muted-foreground text-xs">
              {t("providers.opencodeImport.summary", { created, updated })}
            </p>
            <ImportPreviewList entries={phase.entries} preview />
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 flex items-start gap-2 rounded-md border p-2.5 text-sm">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
              <span>
                {t("providers.opencodeImport.report.imported", {
                  count: phase.imported,
                })}
              </span>
            </div>
            <ImportPreviewList entries={phase.entries} preview={false} />
          </div>
        )}

        <DialogFooter>
          {phase.kind === "ready" && phase.entries.length > 0 ? (
            <>
              <Button variant="outline" onClick={close}>
                {t("common.cancel")}
              </Button>
              <Button onClick={onConfirm} disabled={importing}>
                {importing ? <Loader2 className="animate-spin" /> : <Upload />}
                {t("providers.opencodeImport.import")}
              </Button>
            </>
          ) : (
            <Button onClick={close}>{t("common.close")}</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** 条目列表：名称 + 端点（mono 截断）+ 徽标组。preview=true 时额外显示
 *  「含密钥/无密钥」与「新建/更新」徽标；结果视图只留名称与端点（密钥信息
 *  在预览环节已完成告知，结果页保持干净）。 */
function ImportPreviewList({
  entries,
  preview,
}: {
  entries: OpenCodeImportPreviewEntry[]
  preview: boolean
}) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col">
      {entries.map((e) => (
        <div
          key={e.key}
          className="flex items-center justify-between gap-3 border-b border-border py-1.5 text-sm last:border-b-0"
        >
          <span className="min-w-0">
            <span className="truncate font-medium">{e.name}</span>
            {e.baseUrl ? (
              <span className="text-muted-foreground ml-2 truncate font-mono text-xs">
                {e.baseUrl}
              </span>
            ) : null}
          </span>
          <span className="flex shrink-0 items-center gap-1.5">
            {preview ? (
              <>
                <Badge
                  variant={e.hasSecret ? "secondary" : "outline"}
                  className="h-5 shrink-0 px-1.5 font-normal text-[11px]"
                >
                  {e.hasSecret
                    ? t("providers.opencodeImport.hasSecret")
                    : t("providers.opencodeImport.noSecret")}
                </Badge>
                <Badge
                  variant="outline"
                  className="h-5 shrink-0 px-1.5 font-normal text-[11px]"
                >
                  {e.isNew
                    ? t("providers.opencodeImport.badge.new")
                    : t("providers.opencodeImport.badge.update")}
                </Badge>
              </>
            ) : null}
          </span>
        </div>
      ))}
    </div>
  )
}
