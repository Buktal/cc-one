// 「从本机配置文件导入」对话框（泛化自 opencode.json 导入，ADR-0012）：打开时
// 先读该应用的 live 配置文件，列出将导入的供应商（名称 / 端点 / 是否含密钥 /
// 新建或更新），确认后才执行导入，完成后内联展示结果报告（取代原来的直接执行
// + toast）。opencode 是 provider.<key> map（多条）；单激活应用一份 live → 至多
// 一条（claude/codex/gemini/grok）。自包含：内部调 previewLiveImport /
// importProvidersFromLive 两个 mutation。对话框常驻挂载（providers-view 固定
// 渲染），`!open` 只 return null 不卸载——故预览 effect 依赖 `open`：关闭再
// 打开会重新读盘，不残留上一次的旧结果。
//
// 命名时刻（signature）：导入名默认取 base_url 的注册域（后端 host_of），每条
// 目下方展示推导理由「名取自 <url> 的主机名」；行内点击名字即可改名，改后由
// 用户接管（理由行消失），确认导入时通过 nameOverrides 传给后端覆盖推导名。
//
// 视图状态机：loading → missing（文件不存在，带路径）/ error（红块）/ ready 空态
// （无可导入）→ ready 预览（摘要 + 条目列表 + 确认）→ result（成功块 + 列表）。
// 密钥不进预览载荷：条目只有 hasSecret 布尔，密钥值永不跨边界（Rust 侧有防泄漏
// 回归测试锁着）。

import {
  AlertCircle,
  CheckCircle2,
  FileJson,
  FileQuestion,
  Loader2,
  Pencil,
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
import { Input } from "@/components/ui/input"
import {
  groupSnippetCandidates,
  snippetCoveredKeys,
} from "@/features/providers/derive"
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
  // 预览列表里行内改过的名字（key → name），确认导入时传给后端覆盖推导名
  // （单激活 key == name；opencode key = provider.<key>）。
  const [nameOverrides, setNameOverrides] = useState<Record<string, string>>({})
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

  // 打开 → 立即预览（mutation 无缓存，每次打开都是新读盘）。组件常驻挂载，
  // 依赖 `open` 让关闭再打开重新读盘、并清掉上一次的行内改名。preview trigger
  // 引用跨渲染稳定，加进依赖数组无副作用（RTK Query 保证）。
  useEffect(() => {
    if (!open) return
    setPhase({ kind: "loading" })
    setNameOverrides({})
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
  }, [open, app, preview])

  async function onConfirm() {
    if (phase.kind !== "ready") return
    const result = await importFromLive({ app, nameOverrides })
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
          <p className="bg-muted/50 text-muted-foreground flex items-start gap-1.5 rounded-md px-3 py-2 text-xs">
            <FileQuestion className="mt-0.5 size-3.5 shrink-0" />
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
            <ImportPreviewList
              entries={phase.entries}
              preview
              overrides={nameOverrides}
              onRename={(key, name) =>
                setNameOverrides((prev) => ({ ...prev, [key]: name }))
              }
            />
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
            <ImportPreviewList
              entries={phase.entries}
              preview={false}
              overrides={nameOverrides}
            />
            {/* T6：单激活应用导入后检测到可共享键（且片段缺该键）→ 非静默提示
                「提取为通用片段」。用户确认才提取（ADR-0012）。opencode 无候选
                → 不显示。候选按「端点 / 模型 / 行为开关」人话分组展示。 */}
            {pendingCandidates.length > 0 ? (
              <div className="bg-muted/40 rounded-md border border-border p-2.5">
                <div className="flex items-start justify-between gap-2">
                  <span className="text-muted-foreground text-xs">
                    {t("providers.liveImport.extractHint", {
                      count: pendingCandidates.length,
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
                <CandidateGroups keys={pendingCandidates} />
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

/** 可共享配置候选的三组人话分组（端点 / 模型 / 行为开关）。分组只改展示
 *  结构，提取时仍提取全部候选（后端过滤凭据）。 */
function CandidateGroups({ keys }: { keys: string[] }) {
  const { t } = useTranslation()
  const groups = groupSnippetCandidates(keys)
  const rows: Array<[string, string[]]> = []
  if (groups.endpoint.length > 0) {
    rows.push([t("providers.liveImport.extractGroups.endpoint"), groups.endpoint])
  }
  if (groups.model.length > 0) {
    rows.push([t("providers.liveImport.extractGroups.model"), groups.model])
  }
  if (groups.behavior.length > 0) {
    rows.push([t("providers.liveImport.extractGroups.behavior"), groups.behavior])
  }
  return (
    <div className="mt-1.5 flex flex-col gap-0.5">
      {rows.map(([label, ks]) => (
        <div key={label} className="flex items-baseline gap-1.5 text-xs">
          <span className="text-muted-foreground shrink-0">{label}</span>
          <span className="min-w-0 truncate font-mono">{ks.join(", ")}</span>
        </div>
      ))}
    </div>
  )
}

/** 条目行：名字（可点击改名）+ 推导理由行（等宽、截断）+ 徽标组。preview=true
 *  时显示「含密钥/无密钥」与「新建/更新」徽标且名字可改；结果视图只读（密钥
 *  信息在预览环节已完成告知，结果页保持干净，名字已按覆盖值落库）。行内改名后
 *  理由行消失——名字由用户接管，不再需要解释它的来处。 */
function EntryRow({
  entry: e,
  preview,
  name,
  renamed,
  onRename,
}: {
  entry: LiveImportPreviewEntry
  preview: boolean
  /** 显示名 = 行内覆盖名，未改过则是后端推导名。 */
  name: string
  /** 名字是否被用户行内改过（overrides 里有此 key）。 */
  renamed: boolean
  onRename: (name: string) => void
}) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState("")

  function commit() {
    const trimmed = draft.trim()
    if (trimmed) onRename(trimmed)
    setEditing(false)
  }

  return (
    <div className="group/item flex items-start justify-between gap-3 border-b border-border py-1.5 text-sm last:border-b-0">
      <div className="min-w-0 flex-1">
        {editing ? (
          <Input
            value={draft}
            autoFocus
            onFocus={(ev) => ev.currentTarget.select()}
            onChange={(ev) => setDraft(ev.target.value)}
            onBlur={commit}
            onKeyDown={(ev) => {
              if (ev.key === "Enter") {
                ev.preventDefault()
                commit()
              } else if (ev.key === "Escape") {
                // 清空 draft：Escape 后 input 卸载，若随后 blur 派发，commit
                // 读到空值 → 只退出编辑不改名（放弃本次修改）。
                setEditing(false)
                setDraft("")
              }
            }}
            className="h-6 text-xs"
            aria-label={t("providers.liveImport.col.name")}
          />
        ) : (
          <button
            type="button"
            onClick={() => {
              setDraft(name)
              setEditing(true)
            }}
            disabled={!preview}
            className="text-foreground flex max-w-full items-center gap-1 truncate rounded-sm font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:hover:no-underline"
            title={preview ? t("providers.liveImport.renameHint") : undefined}
          >
            {name}
            <Pencil className="text-muted-foreground size-3 shrink-0 opacity-0 transition-opacity group-hover/item:opacity-100" />
          </button>
        )}
        {!editing && !renamed && e.baseUrl ? (
          <div className="text-muted-foreground truncate font-mono text-xs">
            {t("providers.liveImport.nameFrom", { url: e.baseUrl })}
          </div>
        ) : null}
      </div>
      <span className="flex shrink-0 items-center gap-1.5 pt-0.5">
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
  )
}

/** 条目列表：名称（行内可改名）+ 推导理由行 + 徽标组。overrides = 预览阶段
 *  行内改过的名字（key → name）；结果视图沿用（导入已按覆盖后的名字落库）。 */
function ImportPreviewList({
  entries,
  preview,
  overrides,
  onRename,
}: {
  entries: LiveImportPreviewEntry[]
  preview: boolean
  overrides: Record<string, string>
  onRename?: (key: string, name: string) => void
}) {
  return (
    <div className="flex flex-col">
      {entries.map((e) => (
        <EntryRow
          key={e.key}
          entry={e}
          preview={preview}
          name={overrides[e.key] ?? e.name}
          renamed={overrides[e.key] !== undefined}
          onRename={(name) => onRename?.(e.key, name)}
        />
      ))}
    </div>
  )
}
