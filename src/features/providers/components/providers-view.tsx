// Providers view (供应商): the mid-view preset chip selector, plus the local
// provider list with drag-to-reorder, add / edit (via Sheet) / delete. Rows are
// a Card list (not a <table>) so the whole row can act as the dnd-kit drag
// handle, matching the group sidebar's reorder interaction. Empty state nudges
// toward the preset picker above — clicking a chip opens the form pre-filled
// from that preset (the preset is only a starting point).

import { PointerActivationConstraints } from "@dnd-kit/dom"
import {
  DragDropProvider,
  type DragEndEvent,
  PointerSensor,
} from "@dnd-kit/react"
import { useSortable } from "@dnd-kit/react/sortable"
import {
  AlertTriangle,
  Copy,
  Database,
  Download,
  Info,
  Pencil,
  Plus,
  Search,
  Trash2,
  Upload,
  X,
} from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useAddProviderToLiveMutation,
  useDeleteProviderMutation,
  useGetActiveProviderQuery,
  useRemoveProviderFromLiveMutation,
  useSwitchProviderMutation,
} from "@/app/store/api"
import { ConfirmDialog } from "@/components/confirm-dialog"
import { EmptyState } from "@/components/empty-state"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
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
  filterProviders,
  providerEndpoint,
  providerLiveManaged,
  providerMissingRequired,
  providerModel,
} from "@/features/providers/derive"
import { useProvidersBrowser } from "@/features/providers/use-providers-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { usePersistedState } from "@/lib/persistence"
import { cn } from "@/lib/utils"

import type {
  App,
  Provider,
  ProviderCategory,
} from "@/types/generated/bindings"
import { CcSwitchImportDialog } from "./cc-switch-import-dialog"
import { CommonConfigSnippetCard } from "./common-config-snippet-card"
import { ImportSourceDialog } from "./import-source-dialog"
import { LiveImportDialog } from "./live-import-dialog"
import { ProviderFormSheet } from "./provider-form-sheet"
import {
  ProviderTransferDialog,
  type TransferKind,
} from "./provider-transfer-dialog"

// 顶部分段控件的三档应用——顺序即显示顺序。各应用拥有独立的供应商池、
// 激活状态、预设清单与通用配置片段。
const APPS: App[] = ["claude", "codex", "gemini", "grok", "opencode"]

export function ProvidersView() {
  const { t } = useTranslation()
  // 当前选中的应用跨重启记忆（与侧栏折叠同一套 usePersistedState 机制）。
  const [app, setApp] = usePersistedState<App>("cc-one:providers-app", "claude")
  const {
    providers,
    isLoading,
    onReorder,
    exportProviders,
    importProviders,
    transferring,
  } = useProvidersBrowser(app)
  const { data: activeProvider } = useGetActiveProviderQuery(app)
  const [remove] = useDeleteProviderMutation()
  const [switchProvider] = useSwitchProviderMutation()
  // opencode 附加模式：加入 / 移出 live 配置（opencode.json）。单激活四 app
  // 不用这两个。「从本机配置文件导入」走 LiveImportDialog（预览式弹窗）。
  const [addProviderToLive] = useAddProviderToLiveMutation()
  const [removeProviderFromLive] = useRemoveProviderFromLiveMutation()
  const runWithToast = useMutateWithToast()

  const [sheetOpen, setSheetOpen] = useState(false)
  const [editing, setEditing] = useState<Provider | null>(null)
  // 待删除供应商（非 null 弹确认框）；删除成功才清空——保持打开直到完成。
  const [deleting, setDeleting] = useState<Provider | null>(null)
  const [deletingBusy, setDeletingBusy] = useState(false)
  async function onConfirmDelete() {
    if (!deleting) return
    setDeletingBusy(true)
    const ok = await onDelete(deleting)
    // 成功也重置 busy：关闭由 prop（setDeleting(null)）驱动，不触发
    // onOpenChange（Radix 只在用户交互时回调）——保持 true 会让下次打开
    // 弹窗时按钮一直转圈。关闭动画里闪回一帧按钮文案无关紧要。
    setDeletingBusy(false)
    if (ok) setDeleting(null)
  }
  const [transfer, setTransfer] = useState<TransferKind | null>(null)
  const [ccswitchOpen, setCcswitchOpen] = useState(false)
  // 「从本机配置文件导入」预览弹窗（全部应用，ADR-0012）。
  const [liveImportOpen, setLiveImportOpen] = useState(false)
  // 「导入」来源选择弹窗：CC-Switch / 本机配置 / CC One 备份，点卡片进入对应
  // 流程（全部导入来源收进一个入口，来源不用展开菜单就一眼可见）。
  const [importSourceOpen, setImportSourceOpen] = useState(false)
  const [query, setQuery] = useState("")
  // 切换确认对话框：切到缺必填项（端点/key/模板变量）的供应商前先问一句，
  // 用户确认后照切不误（「我已经知道它缺东西，就是要切」）。
  /** 缺必填项时挂起的写盘动作：确认后执行。kind 区分单激活「切换」与附加
   *  模式「加入 live」（opencode——附加模式同样可能缺 key/端点，加入后不可用，
   *  确认框两者共用）。 */
  const [confirmPending, setConfirmPending] = useState<{
    provider: Provider
    kind: "switch" | "addToLive"
  } | null>(null)
  // opencode 解释条：切走再切回 opencode 时重新弹出（selectApp 里重置）。
  const [bannerDismissed, setBannerDismissed] = useState(false)

  // Whole-row drag handle: 6px of movement before a press becomes a drag —
  // clicks keep opening the edit sheet; moves reorder. Same constraints as the
  // group sidebar.
  const sensors = [
    PointerSensor.configure({
      activationConstraints: () => [
        new PointerActivationConstraints.Distance({ value: 6 }),
      ],
      preventActivation: () => false,
    }),
  ]

  function handleDragEnd(event: DragEndEvent): void {
    if (event.canceled) return
    const sourceId = event.operation.source?.id
    const targetId = event.operation.target?.id
    if (sourceId == null || targetId == null || sourceId === targetId) return
    onReorder(String(sourceId), String(targetId))
  }

  function openNew() {
    setEditing(null)
    setSheetOpen(true)
  }
  function openEdit(p: Provider) {
    setEditing(p)
    setSheetOpen(true)
  }
  /** 复制：副本走编辑通道但 id 清空（保存即新建），名称带「副本」后缀。
   *  快照/meta 原样复制——模板变量值（meta 里）随之带进副本表单。 */
  function openDuplicate(p: Provider) {
    setEditing({
      ...p,
      id: "",
      name: `${p.name}${t("providers.copyNameSuffix")}`,
    })
    setSheetOpen(true)
  }
  // 返回成功与否：删除确认框保持打开直到成功（成功才关），失败留在框内重试。
  async function onDelete(p: Provider): Promise<boolean> {
    return await runWithToast(
      remove,
      { app, id: p.id },
      {
        success: { key: "providers.toast.deleted", vars: { name: p.name } },
        failed: { key: "providers.toast.deleteFailed" },
      },
    )
  }

  async function doSwitch(p: Provider) {
    await runWithToast(
      switchProvider,
      { app, id: p.id },
      {
        success: { key: "providers.toast.switched", vars: { name: p.name } },
        failed: { key: "providers.toast.switchFailed" },
      },
    )
  }

  /** 切换入口：缺必填项 → 先弹确认；齐全 → 直接切。 */
  function onSwitch(p: Provider) {
    const missing = providerMissingRequired(p)
    if (missing.length > 0) setConfirmPending({ provider: p, kind: "switch" })
    else void doSwitch(p)
  }

  function confirmMissing(): string[] {
    return confirmPending
      ? providerMissingRequired(confirmPending.provider)
      : []
  }

  // opencode 是附加模式（多供应商共存于 opencode.json，无唯一激活）：行交互走
  // 「加入 / 移出 live」而非「切换」，列表无「当前使用」标记，通用配置片段
  // 无处合并 → 不显示。单激活四 app 恒 false。
  const isAdditive = app === "opencode"

  /** 切换 app：重置 opencode 解释条（切走再切回重新提示）。 */
  function selectApp(a: App) {
    setApp(a)
    setBannerDismissed(false)
  }

  async function doAddToLive(p: Provider) {
    await runWithToast(
      addProviderToLive,
      { app, id: p.id },
      {
        success: { key: "providers.toast.enabledIn", vars: { name: p.name } },
        failed: { key: "providers.toast.enableFailed" },
      },
    )
  }

  /** 附加模式「加入 live」入口：与切换同一必填项检查——缺 → 先弹确认。 */
  function onAddToLive(p: Provider) {
    const missing = providerMissingRequired(p)
    if (missing.length > 0) {
      setConfirmPending({ provider: p, kind: "addToLive" })
      return
    }
    void doAddToLive(p)
  }
  async function onRemoveFromLive(p: Provider) {
    await runWithToast(
      removeProviderFromLive,
      { app, id: p.id },
      {
        success: {
          key: "providers.toast.disabledIn",
          vars: { name: p.name },
        },
        failed: { key: "providers.toast.disableFailed" },
      },
    )
  }
  const visibleProviders = filterProviders(providers, query)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      {/* Top bar: app chips wrap inside the fieldset; the action group
        (数据菜单 + 新增) pins right via ml-auto and drops to its own
        right-aligned line on narrow containers, so buttons never scatter
        between the chips. */}
      <div className="flex flex-wrap items-center gap-2">
        <fieldset
          aria-label={t("providers.appLabel")}
          className="m-0 flex flex-wrap gap-1 rounded-lg border p-0.5"
        >
          {APPS.map((a) => (
            <Button
              key={a}
              size="sm"
              variant="ghost"
              onClick={() => selectApp(a)}
              className={cn(
                "rounded-md",
                a === app &&
                  // 激活: 反色块 —— 亮色纯白 (bg-card) 浮在浅底上、暗色
                  // 纯黑 (#0d0d0f) 按进渐变背景; 边框 border-border 跟随
                  // 明暗 (亮深灰 / 暗浅白)。分段控件不套品牌 tint。
                  "border-border bg-card text-foreground hover:bg-card font-medium border",
              )}
            >
              {t(`providers.app.${a}`)}
            </Button>
          ))}
        </fieldset>
        <div className="ml-auto flex gap-2">
          {/* 按方向分类：「导入」弹来源选择框（CC-Switch 迁移 / 本机配置文件 /
              CC One 备份恢复一眼可见，点卡片进入对应流程），「导出」是单一
              动作直接按钮。主操作「新增」仍为 primary。 */}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setImportSourceOpen(true)}
          >
            <Upload />
            {t("providers.importMenu.label")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setTransfer("export")}
          >
            <Download />
            {t("providers.exportMenu.label")}
          </Button>
          <Button size="sm" onClick={openNew}>
            <Plus />
            {t("providers.add")}
          </Button>
        </div>
      </div>

      {isAdditive && !bannerDismissed ? (
        // opencode 附加模式说明：多供应商可同时启用，启用即生效，无需切换。
        // 与 AppMark 描边方块呼应，让附加模式与单激活的差异在顶栏就可预期。
        <div className="border-border bg-card flex items-center gap-2 rounded-lg border px-3 py-2 text-xs">
          <Info className="text-foreground size-3.5 shrink-0" />
          <span className="text-foreground flex-1">
            {t("providers.opencode.bannerBody")}
          </span>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-6 shrink-0"
            aria-label={t("common.close")}
            onClick={() => setBannerDismissed(true)}
          >
            <X />
          </Button>
        </div>
      ) : null}
      {/* 列表卡片不再 flex-1 撑满剩余高度：上方还有通用配置卡片，高度不固
          定，内部滚动会把下面的内容挤出视口且无法滚到。整页自然排布，由
          Shell 的滚动容器统一滚动。预设已退居为新增流程的左侧面板（见下）。 */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between gap-2">
          <CardTitle>
            {t("providers.title")}
            {/* 过滤前总数——搜索时数字稳定，无匹配由 noMatch 空态表达。 */}
            <span className="text-muted-foreground ml-2 text-xs font-normal">
              {providers.length}
            </span>
          </CardTitle>
          <div className="relative w-56">
            <Search className="text-muted-foreground absolute top-1/2 left-2.5 size-4 -translate-y-1/2" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("providers.searchPlaceholder")}
              aria-label={t("providers.searchAria")}
              className="pl-8"
            />
          </div>
        </CardHeader>
        <CardContent className="flex flex-col">
          {isLoading ? (
            <div className="text-muted-foreground py-12 text-center text-sm">
              {t("common.loading")}
            </div>
          ) : visibleProviders.length === 0 ? (
            query.trim() ? (
              <EmptyState icon={Search} title={t("providers.noMatch")} />
            ) : (
              <EmptyState
                icon={Database}
                title={t("providers.empty")}
                description={t("providers.emptyDesc")}
                action={{ label: t("providers.add"), onClick: openNew }}
              />
            )
          ) : (
            <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
              {/* 每行是独立 grid 容器——列宽必须全部由模板决定，不能随行内容
                  漂移：分类列固定 8rem（容纳最长的 "Cloud provider"）、操作列
                  固定 10rem（与下方 w-40 占位一致）、端点/模型列 minmax(0,…)
                  可收缩截断。任何 auto 列都会让 fr 分配逐行不同，三列起点错乱
                  （英文下尤其明显）。与下方 ProviderRow 的模板保持一致。 */}
              {/* 列头 span 与数据列同规则 min-w-0 + truncate：窄窗压窄
                  minmax(0,…) 轨道时，en/ja 长列头（如「エンドポイント」）若
                  不截断会溢进相邻列，与下方截断的行内容错位。 */}
              <div className="text-muted-foreground grid grid-cols-[minmax(10rem,1.2fr)_8rem_minmax(0,1.4fr)_minmax(0,1fr)_10rem] gap-3 px-4 pb-2 text-xs">
                <span className="min-w-0 truncate">
                  {t("providers.col.name")}
                </span>
                <span className="min-w-0 truncate">
                  {t("providers.col.category")}
                </span>
                <span className="min-w-0 truncate">
                  {t("providers.col.endpoint")}
                </span>
                <span className="min-w-0 truncate">
                  {t("providers.col.model")}
                </span>
                <span className="w-40" />
              </div>
              <DragDropProvider sensors={sensors} onDragEnd={handleDragEnd}>
                {visibleProviders.map((p, i) => (
                  <ProviderRow
                    key={p.id}
                    provider={p}
                    index={i}
                    additive={isAdditive}
                    isActive={activeProvider?.id === p.id}
                    liveManaged={providerLiveManaged(p)}
                    onEdit={() => openEdit(p)}
                    onDuplicate={() => openDuplicate(p)}
                    onDelete={() => setDeleting(p)}
                    onSwitch={() => onSwitch(p)}
                    onAddToLive={() => void onAddToLive(p)}
                    onRemoveFromLive={() => void onRemoveFromLive(p)}
                  />
                ))}
              </DragDropProvider>
            </div>
          )}
        </CardContent>
      </Card>

      {/* 通用配置片段：按应用分派合并层（ADR-0010）——claude/gemini 在
          settings_config 层并入、codex/grok 在写盘层补缺失进 live 文件。卡片只对
          「切换时真正合并片段」的应用显示（#47 原则：不给配了不生效的应用展示控件）。
          opencode 附加模式无「切换」概念，不显示。 */}
      {app === "claude" ||
      app === "codex" ||
      app === "gemini" ||
      app === "grok" ? (
        <CommonConfigSnippetCard app={app} />
      ) : null}

      <ProviderFormSheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        editing={editing}
        app={app}
        onSaved={() => setSheetOpen(false)}
        onResetEditing={() => setEditing(null)}
      />
      <Dialog
        open={confirmPending !== null}
        onOpenChange={(open) => !open && setConfirmPending(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("providers.switchConfirm.title")}</DialogTitle>
            <DialogDescription>
              {t("providers.switchConfirm.description", {
                name: confirmPending?.provider.name ?? "",
                missing: confirmMissing()
                  .map((m) => t(`providers.switchConfirm.missing.${m}`))
                  .join(", "),
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmPending(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              onClick={() => {
                const pending = confirmPending
                setConfirmPending(null)
                if (!pending) return
                if (pending.kind === "switch") void doSwitch(pending.provider)
                else void doAddToLive(pending.provider)
              }}
            >
              {t("providers.switchConfirm.switch")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <ProviderTransferDialog
        kind={transfer}
        transferring={transferring}
        onExport={exportProviders}
        onImport={importProviders}
        onOpenChange={setTransfer}
      />
      <CcSwitchImportDialog
        open={ccswitchOpen}
        onOpenChange={setCcswitchOpen}
        app={app}
      />
      <LiveImportDialog
        open={liveImportOpen}
        onOpenChange={setLiveImportOpen}
        app={app}
      />
      <ImportSourceDialog
        open={importSourceOpen}
        onOpenChange={setImportSourceOpen}
        onImportCcSwitch={() => setCcswitchOpen(true)}
        onImportLive={() => setLiveImportOpen(true)}
        onImportBackup={() => setTransfer("import")}
      />

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          // busy 由 onConfirmDelete 在成功/失败路径自行重置（prop 驱动的
          // 关闭不触发本回调）；用户取消时 busy 恒为 false（删除中按钮已
          // disabled），无需在此清理。
          if (!open) setDeleting(null)
        }}
        title={t("confirm.deleteTitle", { name: deleting?.name ?? "" })}
        description={t("providers.confirm.deleteDesc", {
          name: deleting?.name ?? "",
        })}
        confirmLabel={t("common.delete")}
        busy={deletingBusy}
        onConfirm={() => void onConfirmDelete()}
      />
    </div>
  )
}

/** 分类 → 色变量名。官方用固定语义蓝（--cat-official，不随皮肤漂移——chart
 *  四桶是数据语义色，crimson/sage 等皮肤里 cache-create 是淡蓝/橙/黄，借它
 *  当「官方」换肤即变色甚至发白）；其余三色沿用 chart 色跟随皮肤。
 *  official 与 cn_official 同文案同色。 */
const CATEGORY_COLOR: Record<ProviderCategory, string> = {
  official: "cat-official",
  cn_official: "cat-official",
  aggregator: "chart-cache-read",
  cloud_provider: "chart-input",
  custom: "chart-output",
}

/** 分类徽标样式：分类色 tint 底 + 边框（内联 style 保证覆盖 outline variant
 *  的 border-border——tailwind 类冲突时谁后生成不可靠）。官方固定蓝饱和度
 *  高，背景 20% 即「淡蓝透白」的轻底，边框 60% 保证 hover 行上可辨；其余
 *  分类 18%/40% 保持既有观感。深色模式下 tint 混暗底自然压深，两种模式都
 *  成立。 */
function categoryBadgeStyle(category: ProviderCategory) {
  const isOfficial = category === "official" || category === "cn_official"
  const color = CATEGORY_COLOR[category]
  const strength = isOfficial ? 20 : 18
  const border = isOfficial ? 60 : 40
  return {
    backgroundColor: `color-mix(in srgb, var(--${color}) ${strength}%, transparent)`,
    borderColor: `color-mix(in srgb, var(--${color}) ${border}%, transparent)`,
  }
}

function ProviderRow({
  provider: p,
  index,
  additive,
  isActive,
  liveManaged,
  onEdit,
  onDuplicate,
  onDelete,
  onSwitch,
  onAddToLive,
  onRemoveFromLive,
}: {
  provider: Provider
  index: number
  /** 附加模式（opencode）：走「加入/移出 live」而非「切换/使用中」。 */
  additive: boolean
  isActive: boolean
  /** 附加模式：该供应商是否已写进 opencode.json（meta.liveManaged）。 */
  liveManaged: boolean
  onEdit: () => void
  onDuplicate: () => void
  onDelete: () => void
  onSwitch: () => void
  onAddToLive: () => void
  onRemoveFromLive: () => void
}) {
  const { t } = useTranslation()
  const { ref, isDragging } = useSortable({ id: p.id, index })
  const endpoint = providerEndpoint(p)
  const model = providerModel(p)
  // 缺必填项（端点 / key / 未物化模板变量）：提前到列表层可见，hover 列明细，
  // 不必等点「切换 / 加入 live」才在确认框里发现。official / cloud_provider 走
  // 默认端点或模板变量认证，不要求端点/key（见 providerMissingRequired）。
  const missingRequired = providerMissingRequired(p)
  return (
    <div
      ref={ref}
      className={cn(
        // 与列头模板一致（分类 8rem / 操作 10rem 固定）：行是独立 grid 容器，
        // 任何 auto 列都会让列宽逐行漂移——见列头注释。group 供行 hover 时给
        // 操作按钮浮出白底（见操作区注释）。
        "group grid grid-cols-[minmax(10rem,1.2fr)_8rem_minmax(0,1.4fr)_minmax(0,1fr)_10rem] items-center gap-3 rounded-lg px-4 py-2 transition-colors",
        // 当前使用：品牌色背景 + 左侧色条，作为列表的视觉锚点（取代旧的独立
        // 「当前使用」卡片）。不置顶——保留用户拖拽自定义的顺序。
        isActive
          ? "bg-accent-tint/60 shadow-[inset_2px_0_0_var(--accent-brand)]"
          : "hover:bg-hover",
        isDragging && "bg-hover opacity-70 shadow-sm",
      )}
    >
      <span className="flex min-w-0 items-center gap-2">
        {/* 缺必填项（端点/key/模板变量）提醒放名称最左，缺了才显示；hover 列
            明细。official / cloud_provider 不要求端点/key（见
            providerMissingRequired），它们不显示此图标。 */}
        {missingRequired.length > 0 ? (
          <Tooltip>
            <TooltipTrigger
              render={<span className="inline-flex shrink-0 cursor-help" />}
            >
              <AlertTriangle className="size-3.5 shrink-0 text-amber-500" />
            </TooltipTrigger>
            <TooltipContent>
              {t("providers.row.missingRequired", {
                missing: missingRequired
                  .map((m) => t(`providers.switchConfirm.missing.${m}`))
                  .join(", "),
              })}
            </TooltipContent>
          </Tooltip>
        ) : null}
        <span className="truncate font-medium">{p.name}</span>
        {additive && liveManaged ? (
          /* 「已加入 live」状态徽标放名字旁而非操作区：操作列固定 10rem，
             徽标若留在按钮组会把操作列撑宽、列宽逐行漂移（见列头注释）。 */
          <Badge
            variant="outline"
            className="h-5 shrink-0 px-1.5 text-[11px] font-normal"
          >
            {t("providers.live.enabled")}
          </Badge>
        ) : null}
      </span>
      {/* 分类徽标按分类固定四色（chart 数据色：差异大、非黑白灰、换肤跟随）：
          行 hover 用 bg-muted，灰底 secondary 徽标会融进行背景消失；分类色的
          12% tint 底 + 边框在 hover 下依然可辨。official 与 cn_official 同文案
          同色。色点从行首移进徽标——色点在名字前只是装饰，进徽标才绑定语义。 */}
      <Badge
        variant="outline"
        className="h-5 shrink-0 gap-1 px-1.5 font-normal text-[11px]"
        style={categoryBadgeStyle(p.category)}
      >
        <span
          aria-hidden
          className="size-1.5 shrink-0 rounded-full"
          style={{
            backgroundColor: `var(--${CATEGORY_COLOR[p.category]})`,
          }}
        />
        {t(`providers.category.${p.category}`)}
      </Badge>
      {endpoint ? (
        <Tooltip>
          <TooltipTrigger
            render={
              <span className="text-muted-foreground truncate font-mono text-xs">
                {endpoint}
              </span>
            }
          />
          <TooltipContent>{endpoint}</TooltipContent>
        </Tooltip>
      ) : (
        <span className="text-muted-foreground truncate font-mono text-xs">
          —
        </span>
      )}
      <span className="text-muted-foreground truncate text-xs">
        {model || "—"}
      </span>
      {/* 行内按钮：行 hover（bg-hover）时灰边框会融进行背景只剩文字——switch
          按钮（行内主操作）行 hover 时边框换 accent 色保持可辨，编辑/删除是
          ghost 图标按钮，融入可接受。按钮自身 hover 用 hover:!bg-accent-brand/25
          （主题色，比 accent-tint 浓），! 确保覆盖 Button 默认的 hover:bg-hover。 */}
      <div className="flex justify-end gap-1">
        {additive ? (
          // 附加模式（opencode）：按 liveManaged 显示「移出」或「加入」，不走
          // 单激活的「切换 / 使用中」。已加入状态徽标在名字列（见上）。
          liveManaged ? (
            <Button
              variant="outline"
              size="sm"
              onClick={onRemoveFromLive}
              className="shrink-0 group-hover:border-accent-brand/60 hover:!bg-accent-brand/25"
            >
              {t("providers.live.disable")}
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onClick={onAddToLive}
              className="shrink-0 group-hover:border-accent-brand/60 hover:!bg-accent-brand/25"
            >
              {t("providers.live.enable")}
            </Button>
          )
        ) : isActive ? (
          // h-8 与同组的「切换」Button size="sm" 及 icon-sm 图标按钮同高：
          // 两态切换时该槽位控件高度不变（h-6 会在 24/32px 间跳变）。
          <Badge
            variant="outline"
            className="h-8 gap-1 shrink-0 px-2 font-normal text-[11px]"
          >
            <span
              aria-hidden
              className="bg-accent-brand size-1.5 rounded-full"
            />
            {t("providers.active.inUse")}
          </Badge>
        ) : (
          <Button
            variant="outline"
            size="sm"
            onClick={onSwitch}
            className="shrink-0 group-hover:border-accent-brand/60 hover:!bg-accent-brand/25"
          >
            {t("providers.switch")}
          </Button>
        )}
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onEdit}
                aria-label={t("common.edit")}
                className="hover:!bg-accent-brand/25"
              />
            }
          >
            <Pencil />
          </TooltipTrigger>
          <TooltipContent>{t("common.edit")}</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onDuplicate}
                aria-label={t("common.copy")}
                className="hover:!bg-accent-brand/25"
              />
            }
          >
            <Copy />
          </TooltipTrigger>
          <TooltipContent>{t("common.copy")}</TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                variant="ghost"
                size="icon-sm"
                onClick={onDelete}
                aria-label={t("common.delete")}
                className="hover:!bg-accent-brand/25"
              />
            }
          >
            <Trash2 />
          </TooltipTrigger>
          <TooltipContent>{t("common.delete")}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  )
}
