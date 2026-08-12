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
  ArrowRightLeft,
  Download,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Upload,
} from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useAddProviderToLiveMutation,
  useDeleteProviderMutation,
  useGetActiveProviderQuery,
  useImportProvidersFromLiveMutation,
  useRemoveProviderFromLiveMutation,
  useSwitchProviderMutation,
} from "@/app/store/api"
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
import {
  type ProviderPreset,
  presetsForApp,
} from "@/features/providers/presets"
import { useProvidersBrowser } from "@/features/providers/use-providers-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { usePersistedState } from "@/lib/persistence"
import { cn } from "@/lib/utils"

import type { App, Provider } from "@/types/generated/bindings"
import { CcSwitchImportDialog } from "./cc-switch-import-dialog"
import { CommonConfigSnippetCard } from "./common-config-snippet-card"
import { PresetSidePanel } from "./preset-side-panel"
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
  // opencode 附加模式：加入 / 移出 live 配置（opencode.json），以及从现有
  // opencode.json 反向导入 DB。单激活四 app 不用这三个。
  const [addProviderToLive] = useAddProviderToLiveMutation()
  const [removeProviderFromLive] = useRemoveProviderFromLiveMutation()
  const [importProvidersFromLive] = useImportProvidersFromLiveMutation()
  const runWithToast = useMutateWithToast()

  const [sheetOpen, setSheetOpen] = useState(false)
  // 预设侧栏面板（新增时从左侧滑入）：openNew 时按「该 app 有内置预设」
  // 开启；openEdit / 保存 / 取消时一并关闭。opencode 附加模式无预设 → 恒 false。
  const [presetSheetOpen, setPresetSheetOpen] = useState(false)
  const [editing, setEditing] = useState<Provider | null>(null)
  const [preset, setPreset] = useState<ProviderPreset | null>(null)
  const [transfer, setTransfer] = useState<TransferKind | null>(null)
  const [ccswitchOpen, setCcswitchOpen] = useState(false)
  const [query, setQuery] = useState("")
  // 切换确认对话框：切到缺必填项（端点/key/模板变量）的供应商前先问一句，
  // 用户确认后照切不误（「我已经知道它缺东西，就是要切」）。
  const [confirmSwitch, setConfirmSwitch] = useState<Provider | null>(null)

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
    setPreset(null)
    setSheetOpen(true)
    // 有内置预设的 app：新增时同时弹出左侧预设面板（右侧空白表单 + 左侧预设，
    // 点预设即预填）。opencode 无预设 → 不弹。
    setPresetSheetOpen(presetsForApp(app).length > 0)
  }
  function openFromPreset(p: ProviderPreset) {
    setEditing(null)
    setPreset(p)
    setSheetOpen(true)
  }
  function openEdit(p: Provider) {
    setEditing(p)
    setPreset(null)
    setSheetOpen(true)
    // 编辑已有供应商不挂预设面板（预设只服务新建流程）。
    setPresetSheetOpen(false)
  }
  async function onDelete(p: Provider) {
    await runWithToast(
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
    if (missing.length > 0) setConfirmSwitch(p)
    else void doSwitch(p)
  }

  function confirmMissing(): string[] {
    return confirmSwitch ? providerMissingRequired(confirmSwitch) : []
  }

  // opencode 是附加模式（多供应商共存于 opencode.json，无唯一激活）：行交互走
  // 「加入 / 移出 live」而非「切换」，列表无「当前使用」标记，通用配置片段
  // 无处合并 → 不显示。单激活四 app 恒 false。
  const isAdditive = app === "opencode"

  async function onAddToLive(p: Provider) {
    await runWithToast(
      addProviderToLive,
      { app, id: p.id },
      {
        success: { key: "providers.toast.addedToLive", vars: { name: p.name } },
        failed: { key: "providers.toast.addToLiveFailed" },
      },
    )
  }
  async function onRemoveFromLive(p: Provider) {
    await runWithToast(
      removeProviderFromLive,
      { app, id: p.id },
      {
        success: {
          key: "providers.toast.removedFromLive",
          vars: { name: p.name },
        },
        failed: { key: "providers.toast.removeFromLiveFailed" },
      },
    )
  }
  async function onImportFromLive() {
    // 返回导入条数，手动 toast 带上 count（runWithToast 的 vars 不支持返回值）。
    const result = await importProvidersFromLive(app)
    if (result.error) {
      toast.error(t("providers.toast.importFromLiveFailed"))
    } else {
      toast.success(
        t("providers.toast.importedFromLive", { count: result.data }),
      )
    }
  }

  const visibleProviders = filterProviders(providers, query)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex items-center justify-between gap-2">
        <fieldset
          aria-label={t("providers.appLabel")}
          className="m-0 inline-flex gap-1 rounded-lg border p-0.5"
        >
          {APPS.map((a) => (
            <Button
              key={a}
              size="sm"
              variant={a === app ? "default" : "outline"}
              onClick={() => setApp(a)}
              className="rounded-md"
            >
              {t(`providers.app.${a}`)}
            </Button>
          ))}
        </fieldset>
        <div className="flex gap-2">
          {isAdditive ? (
            <Button
              variant="outline"
              size="sm"
              onClick={() => void onImportFromLive()}
            >
              <RefreshCw />
              {t("providers.live.import")}
            </Button>
          ) : null}
          <Button
            variant="outline"
            size="sm"
            onClick={() => setTransfer("export")}
          >
            <Download />
            {t("providers.transfer.export")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setTransfer("import")}
          >
            <Upload />
            {t("providers.transfer.import")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setCcswitchOpen(true)}
          >
            <ArrowRightLeft />
            {t("providers.ccswitch.button")}
          </Button>
          <Button size="sm" onClick={openNew}>
            <Plus />
            {t("providers.add")}
          </Button>
        </div>
      </div>
      {/* 列表卡片不再 flex-1 撑满剩余高度：上方还有通用配置卡片，高度不固
          定，内部滚动会把下面的内容挤出视口且无法滚到。整页自然排布，由
          Shell 的滚动容器统一滚动。预设已退居为新增流程的左侧面板（见下）。 */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between gap-2">
          <CardTitle>{t("providers.title")}</CardTitle>
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
            <div className="text-muted-foreground py-12 text-center text-sm">
              {query.trim() ? t("providers.noMatch") : t("providers.empty")}
            </div>
          ) : (
            <div className="min-h-0 flex-1 -mr-2.5 overflow-auto pr-2.5">
              <div className="text-muted-foreground grid grid-cols-[minmax(10rem,1.2fr)_auto_1.4fr_1fr_auto] gap-3 px-4 pb-2 text-xs">
                <span>{t("providers.col.name")}</span>
                <span>{t("providers.col.category")}</span>
                <span>{t("providers.col.endpoint")}</span>
                <span>{t("providers.col.model")}</span>
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
                    onDelete={() => void onDelete(p)}
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

      {/* 通用配置片段：切换写盘时合并进受控字段。附加模式（opencode）多供应商
          共存、无「切换」概念，片段无处合并 → 不显示。 */}
      {isAdditive ? null : <CommonConfigSnippetCard app={app} />}

      <ProviderFormSheet
        open={sheetOpen}
        onOpenChange={(open) => {
          setSheetOpen(open)
          if (!open) setPresetSheetOpen(false)
        }}
        editing={editing}
        preset={preset}
        app={app}
        onSaved={() => {
          setSheetOpen(false)
          setPresetSheetOpen(false)
        }}
      />
      {/* 预设侧栏面板：新增模式下从左侧滑入，点预设即预填右侧表单（面板保持
          开，可连续切换预设）。仅在有内置预设的 app 显示——opencode 附加模式
          无预设，open 恒 false。 */}
      <PresetSidePanel
        app={app}
        open={presetSheetOpen && presetsForApp(app).length > 0}
        onSelect={openFromPreset}
        onClose={() => setPresetSheetOpen(false)}
      />
      <Dialog
        open={confirmSwitch !== null}
        onOpenChange={(open) => !open && setConfirmSwitch(null)}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("providers.switchConfirm.title")}</DialogTitle>
            <DialogDescription>
              {t("providers.switchConfirm.description", {
                name: confirmSwitch?.name ?? "",
                missing: confirmMissing()
                  .map((m) => t(`providers.switchConfirm.missing.${m}`))
                  .join(", "),
              })}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmSwitch(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              onClick={() => {
                const p = confirmSwitch
                setConfirmSwitch(null)
                if (p) void doSwitch(p)
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
      <CcSwitchImportDialog open={ccswitchOpen} onOpenChange={setCcswitchOpen} />
    </div>
  )
}

function ProviderRow({
  provider: p,
  index,
  additive,
  isActive,
  liveManaged,
  onEdit,
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
  onDelete: () => void
  onSwitch: () => void
  onAddToLive: () => void
  onRemoveFromLive: () => void
}) {
  const { t } = useTranslation()
  const { ref, isDragging } = useSortable({ id: p.id, index })
  const endpoint = providerEndpoint(p)
  const model = providerModel(p)
  return (
    <div
      ref={ref}
      className={cn(
        "grid grid-cols-[minmax(10rem,1.2fr)_auto_1.4fr_1fr_auto] items-center gap-3 rounded-lg px-4 py-2 transition-colors",
        // 当前使用：品牌色背景 + 左侧色条，作为列表的视觉锚点（取代旧的独立
        // 「当前使用」卡片）。不置顶——保留用户拖拽自定义的顺序。
        isActive
          ? "bg-accent-tint/60 shadow-[inset_2px_0_0_var(--accent-brand)]"
          : "hover:bg-muted",
        isDragging && "bg-muted opacity-70 shadow-sm",
      )}
    >
      <span className="flex min-w-0 items-center gap-2">
        {p.iconColor ? (
          <span
            aria-hidden
            className="size-2 shrink-0 rounded-full"
            style={{ backgroundColor: p.iconColor }}
          />
        ) : null}
        <span className="truncate font-medium">{p.name}</span>
      </span>
      <Badge variant="secondary">{t(`providers.category.${p.category}`)}</Badge>
      <span
        className="text-muted-foreground truncate font-mono text-xs"
        title={endpoint}
      >
        {endpoint || "—"}
      </span>
      <span className="text-muted-foreground truncate text-xs">
        {model || "—"}
      </span>
      <div className="flex justify-end gap-1">
        {additive ? (
          // 附加模式（opencode）：按 liveManaged 显示「加入」或「已加入 + 移出」，
          // 不走单激活的「切换 / 使用中」。
          liveManaged ? (
            <>
              <Badge
                variant="outline"
                className="h-7 shrink-0 px-2 font-normal"
              >
                {t("providers.live.added")}
              </Badge>
              <Button
                variant="outline"
                size="sm"
                onClick={onRemoveFromLive}
                className="shrink-0"
              >
                {t("providers.live.remove")}
              </Button>
            </>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onClick={onAddToLive}
              className="shrink-0"
            >
              {t("providers.live.add")}
            </Button>
          )
        ) : isActive ? (
          <Badge variant="outline" className="h-7 shrink-0 px-2 font-normal">
            {t("providers.active.inUse")}
          </Badge>
        ) : (
          <Button
            variant="outline"
            size="sm"
            onClick={onSwitch}
            className="shrink-0"
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
                onClick={onDelete}
                aria-label={t("common.delete")}
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
