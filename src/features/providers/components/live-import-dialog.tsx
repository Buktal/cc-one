// 「从本机配置文件导入」对话框（泛化自 opencode.json 导入，ADR-0012）：打开时
// 先读该应用的 live 配置文件，列出将导入的供应商（名称 / 端点 / 是否含密钥 /
// 新建或更新），确认后才执行导入，完成后内联展示结果报告（取代原来的直接执行
// + toast）。opencode 是 provider.<key> map（多条）；单激活应用一份 live → 至多
// 一条（claude/codex/gemini/grok）。自包含：内部调 previewLiveImport /
// importProvidersFromLive 两个 mutation，`!open` 直接 unmount 重置状态（与
// cc-switch-import-dialog 同一生命周期）。
//
// 视图状态机：loading → missing（文件不存在，带路径）/ error（红块）/ ready 空态
// （无可导入）→ ready 预览（摘要 + 条目列表 + 确认）→ result（成功块 + 列表）。
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
import { toast } from "sonner"
import {
  useExtractSnippetFromLiveMutation,
  useGetCommonConfigSnippetQuery,
  useImportProvidersFromLiveMutation,
  usePreviewLiveImportMutation,
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
import { snippetCoveredKeys } from "@/features/providers/derive"
import { toStructuredError } from "@/lib/error"

import type { App, LiveImportPreviewEntry } from "@/types/generated/bindings"

type Phase =
  | { kind: "loading" }
  | { kind: "missing"; path: string }
  | { kind: "error"; message: string }
  | { kind: "ready"; entries: LiveImportPreviewEntry[] }
  | { kind: "result"; imported: number; entries: LiveImportPreviewEntry[] }

/** 各应用 live 配置文件名（标题/空态提示用；与后端 live_* 路径一致）。 */
const LIVE_FILE: Record<App, string> = {
  claude: "settings.json",
  codex: "config.toml",
  gemini: ".env",
  grok: "config.toml",
  opencode: "opencode.json",
}

/** mutation 错误 → 人类可读消息（AppError 的 data 优先，与 CC-Switch 导入一致）。 */
function errorMessage(error: unknown): string {
  const structured = toStructuredError(error)
  return structured?.kind === "app"
    ? structured.data
    : (structured?.message ?? String(error))
}

export function LiveImportDialog({
  open,
  onOpenChange,
  app,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  app: App
}) {
  const { t } = useTranslation()
  const [preview] = usePreviewLiveImportMutation()
  const [importFromLive, { isLoading: importing }] =
    useImportProvidersFromLiveMutation()
  const [extractSnippet, { isLoading: extracting }] =
    useExtractSnippetFromLiveMutation()
  const [phase, setPhase] = useState<Phase>({ kind: "loading" })
  // 现有片段（T6 候选过滤用：「片段缺」才提示，ADR-0012）。
  const { data: snippet } = useGetCommonConfigSnippetQuery(app)
  // 导入完成后仍未被现有片段覆盖的候选键。提取只补缺失（ADR-0010），已覆盖
  // 的键提取了零效果——弹了是误导。
  const pendingCandidates =
    phase.kind === "result" && phase.entries[0]
      ? phase.entries[0].snippetCandidates.filter(
          (k) => !snippetCoveredKeys(app, snippet?.content ?? "").has(k),
        )
      : []

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

  /** 「提取为通用片段」（T6）：用户确认后把 live 的可共享键合并进片段。 */
  async function onExtract() {
    const result = await extractSnippet(app)
    if (result.error) {
      toast.error(t("providers.snippet.saveFailed"))
      return
    }
    toast.success(t("providers.liveImport.extractDone"))
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
          <DialogTitle>
            {t("providers.liveImport.title", { file: LIVE_FILE[app] })}
          </DialogTitle>
          <DialogDescription>
            {t("providers.liveImport.hint")}
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
              {t("providers.liveImport.missing", { path: phase.path })}
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
            title={t("providers.liveImport.empty", { file: LIVE_FILE[app] })}
          />
        ) : phase.kind === "ready" ? (
          <div className="flex flex-col gap-3">
            <p className="text-muted-foreground text-xs">
              {t("providers.liveImport.summary", { created, updated })}
            </p>
            <ImportPreviewList entries={phase.entries} preview />
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 flex items-start gap-2 rounded-md border p-2.5 text-sm">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
              <span>
                {t("providers.liveImport.report.imported", {
                  count: phase.imported,
                })}
              </span>
            </div>
            <ImportPreviewList entries={phase.entries} preview={false} />
            {/* T6：单激活应用导入后检测到可共享键（且片段缺该键）→ 非静默提示
                「提取为通用片段」。用户确认才提取（ADR-0012）。opencode 无候选
                → 不显示。 */}
            {pendingCandidates.length > 0 ? (
              <div className="bg-muted/40 flex items-center justify-between gap-2 rounded-md border border-border p-2.5">
                <span className="text-muted-foreground text-xs">
                  {t("providers.liveImport.extractHint", {
                    keys: pendingCandidates.join(", "),
                  })}
                </span>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void onExtract()}
                  disabled={extracting}
                  className="shrink-0"
                >
                  {extracting ? <Loader2 className="animate-spin" /> : null}
                  {t("providers.liveImport.extract")}
                </Button>
              </div>
            ) : null}
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
                {t("providers.liveImport.import")}
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
  entries: LiveImportPreviewEntry[]
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
                    ? t("providers.liveImport.hasSecret")
                    : t("providers.liveImport.noSecret")}
                </Badge>
                <Badge
                  variant="outline"
                  className="h-5 shrink-0 px-1.5 font-normal text-[11px]"
                >
                  {e.isNew
                    ? t("providers.liveImport.badge.new")
                    : t("providers.liveImport.badge.update")}
                </Badge>
              </>
            ) : null}
          </span>
        </div>
      ))}
    </div>
  )
}
