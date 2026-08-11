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
import { Download, Pencil, Plus, Search, Trash2, Upload } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useDeleteProviderMutation,
  useGetActiveProviderQuery,
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import {
  configRoleHasOneM,
  filterProviders,
  MODEL_ROLES,
  providerEndpoint,
  providerMissingRequired,
  providerModel,
} from "@/features/providers/derive"
import type { ProviderPreset } from "@/features/providers/presets"
import { useProvidersBrowser } from "@/features/providers/use-providers-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { usePersistedState } from "@/lib/persistence"
import { cn } from "@/lib/utils"

import type { App, Provider } from "@/types/generated/bindings"
import { CommonConfigSnippetCard } from "./common-config-snippet-card"
import { PresetSelector } from "./preset-selector"
import { ProviderFormSheet } from "./provider-form-sheet"
import {
  ProviderTransferDialog,
  type TransferKind,
} from "./provider-transfer-dialog"

// 顶部分段控件的三档应用——顺序即显示顺序。各应用拥有独立的供应商池、
// 激活状态、预设清单与通用配置片段。
const APPS: App[] = ["claude", "codex", "gemini"]

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
  const { data: activeProvider, isLoading: activeLoading } =
    useGetActiveProviderQuery(app)
  const [remove] = useDeleteProviderMutation()
  const [switchProvider] = useSwitchProviderMutation()
  const runWithToast = useMutateWithToast()

  const [sheetOpen, setSheetOpen] = useState(false)
  const [editing, setEditing] = useState<Provider | null>(null)
  const [preset, setPreset] = useState<ProviderPreset | null>(null)
  const [transfer, setTransfer] = useState<TransferKind | null>(null)
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

  const visibleProviders = filterProviders(providers, query)
  const activeEndpoint = activeProvider ? providerEndpoint(activeProvider) : ""
  const activeModel = activeProvider ? providerModel(activeProvider) : ""
  // 激活供应商哪些模型角色声明了 1M 上下文（角色名 i18n 化，如 "Sonnet"）。
  // 只有带 1M 标记的角色才显示，角色行从快照派生（configRoleFields）。
  const activeOneMRoles = activeProvider
    ? MODEL_ROLES.filter(
        (role) =>
          role.supportsOneM &&
          configRoleHasOneM(activeProvider.settingsConfig, role.id),
      ).map((role) => t(`providers.form.role.${role.id}`))
    : []
  // 光卡「切换」下拉：列出除激活供应商外的所有供应商（搜索过滤后的可见
  // 列表可能为空，因此用全量列表）。
  const switchable = providers.filter((p) => p.id !== activeProvider?.id)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="flex items-center justify-between gap-2">
        <fieldset
          aria-label={t("providers.appLabel")}
          className="m-0 inline-flex rounded-lg border p-0.5"
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
          <Button size="sm" onClick={openNew}>
            <Plus />
            {t("providers.add")}
          </Button>
        </div>
      </div>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">
            {t("providers.active.title")}
          </CardTitle>
        </CardHeader>
        <CardContent className="py-3">
          {activeLoading ? (
            <div className="text-muted-foreground text-sm">
              {t("common.loading")}
            </div>
          ) : activeProvider ? (
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
              <span className="flex items-center gap-2">
                {activeProvider.iconColor ? (
                  <span
                    aria-hidden
                    className="size-2 shrink-0 rounded-full"
                    style={{ backgroundColor: activeProvider.iconColor }}
                  />
                ) : null}
                <span className="font-medium">{activeProvider.name}</span>
              </span>
              <span
                className="text-muted-foreground font-mono text-xs truncate"
                title={activeEndpoint}
              >
                {activeEndpoint || "—"}
              </span>
              <span className="text-muted-foreground text-xs truncate">
                {activeModel || "—"}
              </span>
              {activeOneMRoles.length > 0 ? (
                <span className="text-muted-foreground text-xs">
                  {activeOneMRoles.join(" · ")} 1M
                </span>
              ) : null}
              <div className="ml-auto flex items-center gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => openEdit(activeProvider)}
                >
                  <Pencil />
                  {t("common.edit")}
                </Button>
                <Select
                  value={null}
                  onValueChange={(id) => {
                    const target = providers.find((p) => p.id === id)
                    if (target) onSwitch(target)
                  }}
                  disabled={switchable.length === 0}
                >
                  <SelectTrigger
                    className="w-auto gap-1.5 font-normal"
                    aria-label={t("providers.active.switchTo")}
                  >
                    <SelectValue placeholder={t("providers.active.switchTo")} />
                  </SelectTrigger>
                  <SelectContent>
                    {switchable.map((p) => (
                      <SelectItem key={p.id} value={p.id}>
                        {p.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          ) : (
            <div className="text-muted-foreground text-sm">
              {t("providers.active.none")}
            </div>
          )}
        </CardContent>
      </Card>
      <PresetSelector app={app} onSelect={openFromPreset} />
      {/* 列表卡片不再 flex-1 撑满剩余高度：上方卡片组（active / presets /
          通用配置）高度不固定，内部滚动会把下面的卡片挤出视口且无法滚到。
          整页内容自然排布，由 Shell 的滚动容器统一滚动。 */}
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
                    isActive={activeProvider?.id === p.id}
                    onEdit={() => openEdit(p)}
                    onDelete={() => void onDelete(p)}
                    onSwitch={() => onSwitch(p)}
                  />
                ))}
              </DragDropProvider>
            </div>
          )}
        </CardContent>
      </Card>

      <CommonConfigSnippetCard app={app} />

      <ProviderFormSheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        editing={editing}
        preset={preset}
        app={app}
        onSaved={() => setSheetOpen(false)}
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
    </div>
  )
}

function ProviderRow({
  provider: p,
  index,
  isActive,
  onEdit,
  onDelete,
  onSwitch,
}: {
  provider: Provider
  index: number
  isActive: boolean
  onEdit: () => void
  onDelete: () => void
  onSwitch: () => void
}) {
  const { t } = useTranslation()
  const { ref, isDragging } = useSortable({ id: p.id, index })
  const endpoint = providerEndpoint(p)
  const model = providerModel(p)
  return (
    <div
      ref={ref}
      className={cn(
        "hover:bg-muted grid grid-cols-[minmax(10rem,1.2fr)_auto_1.4fr_1fr_auto] items-center gap-3 rounded-lg px-4 py-2 transition-colors",
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
        {isActive ? (
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
