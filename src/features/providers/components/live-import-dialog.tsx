// 「从本机配置文件导入」对话框（泛化自 opencode.json 导入，ADR-0012）：打开时
// 先读该应用的 live 配置文件，列出将导入的供应商（名称 / 端点 / 是否含密钥 /
// 新建或更新），确认后才执行导入，完成后内联展示结果报告（取代原来的直接执行
// + toast）。opencode 是 provider.<key> map（多条）；单激活应用一份 live → 至多
// 一条（claude/codex/gemini/grok）。自包含：内部调 previewLiveImport /
// importProvidersFromLive 两个 mutation。对话框常驻挂载（providers-view 固定
// 渲染），`!open` 只 return null 不卸载——故预览 effect 依赖 `open`：关闭再
// 打开会重新读盘，不残留上一次的旧结果。
//
// 命名时刻（signature）：单激活应用导入名默认取 base_url 的注册域（后端
// name_from_base_url），条目下方展示推导理由「名取自 <url> 的注册域」（后端
// nameDerivedFromUrl 标志；opencode 名字来自 entry.name / key，不显示理由行）；
// 行内点击名字即可改名，改后由用户接管（理由行消失），确认导入时通过
// nameOverrides 传给后端覆盖推导名。
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
import { useEffect, useRef, useState } from "react"
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
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  groupSnippetCandidates,
  pairModelNameKeys,
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
              <DiscoverPanel
                keys={pendingCandidates}
                extracting={extracting}
                onExtract={() => void onExtract()}
              />
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

// ---- 「发现可共享配置」面板（导入完成后的 coda）----
//
// 呈现为一份 env 检视：区头（标题 + 提取按钮）→ 说明 → 三组小节（端点 / 模型 /
// 行为开关），键名是主体——等宽、完整、按组排列，等宽字体下天然像检视
// settings.json 的 env 区块。模型组按键名配对（*_MODEL 紧跟 *_MODEL_NAME，
// pairModelNameKeys），前缀对齐让「几个角色的默认模型与显示名」一眼可读。
// 端点组单列全宽（唯一键是这份清单的主角），模型/行为两列网格（长键 truncate，
// title 兜底完整键名）。三组色点取 chart 色板（数据色——chrome 单色、数据才有
// 色是项目皮肤铁律，换肤自动跟随）：蓝=端点（连接）、金=模型、绿=行为开关。
// 分组只改展示结构，提取时仍提取全部候选（后端过滤凭据）。

/** 三组小节配置：展示顺序、组标签 key、色点（chart 数据色类）。 */
const GROUPS = [
  { kind: "endpoint", dot: "bg-chart-cache-create" },
  { kind: "model", dot: "bg-chart-input" },
  { kind: "behavior", dot: "bg-chart-cache-read" },
] as const

function DiscoverPanel({
  keys,
  extracting,
  onExtract,
}: {
  keys: string[]
  extracting: boolean
  onExtract: () => void
}) {
  const { t } = useTranslation()
  const groups = groupSnippetCandidates(keys)
  return (
    <div className="rounded-md border border-border">
      <div className="flex items-center justify-between gap-2 border-b border-border px-2.5 py-2">
        <span className="text-sm font-medium">
          {t("providers.liveImport.extractTitle", { count: keys.length })}
        </span>
        <Button
          size="sm"
          variant="outline"
          onClick={onExtract}
          disabled={extracting}
          className="shrink-0"
        >
          {extracting ? <Loader2 className="animate-spin" /> : null}
          {t("providers.liveImport.extract")}
        </Button>
      </div>
      <p className="text-muted-foreground px-2.5 pt-1.5 pb-0.5 text-xs">
        {t("providers.liveImport.extractHint")}
      </p>
      <div className="flex flex-col pb-1">
        {GROUPS.map(({ kind, dot }) => {
          const groupKeys = groups[kind]
          if (groupKeys.length === 0) return null
          const ordered =
            kind === "model" ? pairModelNameKeys(groupKeys) : groupKeys
          return (
            <section key={kind}>
              <div className="flex items-center gap-1.5 px-2.5 pt-1.5 pb-0.5 text-xs">
                <span
                  aria-hidden
                  className={`size-2 shrink-0 rounded-full ${dot}`}
                />
                <span className="text-muted-foreground">
                  {t(`providers.liveImport.extractGroups.${kind}`)}
                </span>
                <span className="text-muted-foreground/60 ml-auto font-mono text-[11px] tabular-nums">
                  {groupKeys.length}
                </span>
              </div>
              <div
                className={
                  kind === "endpoint"
                    ? "flex flex-col px-2.5"
                    : "grid grid-cols-2 gap-x-3 px-2.5"
                }
              >
                {ordered.map((k) => (
                  <Tooltip key={k}>
                    <TooltipTrigger
                      render={
                        <span className="truncate font-mono text-xs leading-6">
                          {k}
                        </span>
                      }
                    />
                    <TooltipContent>{k}</TooltipContent>
                  </Tooltip>
                ))}
              </div>
            </section>
          )
        })}
      </div>
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
  // Escape 放弃编辑的显式守卫：Escape 置位后，即便 input 卸载前 blur 派发，
  // commit 也不会把 Escape 前的草稿提交上去（ref 比依赖「卸载后 blur 不冒泡」
  // 的隐式行为可靠）。
  const cancelEditRef = useRef(false)

  function commit() {
    if (cancelEditRef.current) return
    const trimmed = draft.trim()
    if (trimmed) onRename(trimmed)
    setEditing(false)
  }

  return (
    <div className="flex items-start justify-between gap-3 border-b border-border py-1.5 text-sm last:border-b-0">
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
                // 放弃本次编辑：显式置位守卫，后续任何 blur 都不提交。
                cancelEditRef.current = true
                setEditing(false)
                setDraft("")
              }
            }}
            className="h-6 text-xs"
            aria-label={t("providers.liveImport.col.name")}
          />
        ) : (
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type="button"
                  onClick={() => {
                    cancelEditRef.current = false
                    setDraft(name)
                    setEditing(true)
                  }}
                  disabled={!preview}
                  className="text-foreground flex max-w-full items-center gap-1 truncate rounded-sm font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:hover:no-underline"
                >
                  {name}
                  {/* 编辑图标常显（不 hover 才露）：让「名字可点改名」不需要悬停才发现。
                      结果视图只读（disabled）不显示——不可改名就不画误导性图标。 */}
                  {preview ? (
                    <Pencil className="text-muted-foreground size-3 shrink-0" />
                  ) : null}
                </button>
              }
            />
            {preview ? (
              <TooltipContent>
                {t("providers.liveImport.renameHint")}
              </TooltipContent>
            ) : null}
          </Tooltip>
        )}
        {/* 理由行只在名字确实由 base_url 注册域推导时显示（单激活应用导入）；
            opencode 的名字来自 entry.name / key，显示「名取自 <url>」是撒谎。 */}
        {!editing && !renamed && e.nameDerivedFromUrl ? (
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
