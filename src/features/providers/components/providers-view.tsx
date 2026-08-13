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
  ArrowRightLeft,
  ChevronDown,
  Database,
  Download,
  Info,
  Pencil,
  Plus,
  RefreshCw,
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
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
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

import type { App, Provider } from "@/types/generated/bindings"
import { CcSwitchImportDialog } from "./cc-switch-import-dialog"
import { CommonConfigSnippetCard } from "./common-config-snippet-card"
import { OpencodeImportDialog } from "./opencode-import-dialog"
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
  // 不用这两个。「从 opencode.json 导入」走 OpencodeImportDialog（预览式弹窗）。
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
    // Busy stays true on success: the dialog closes on the same tick, so its
    // closing frame replaces the button — resetting here would flash the
    // spinner back to the label for one frame. Failure resets it for a retry;
    // any close path (cancel / backdrop) resets it in onOpenChange below.
    if (await onDelete(deleting)) {
      setDeleting(null)
    } else {
      setDeletingBusy(false)
    }
  }
  const [transfer, setTransfer] = useState<TransferKind | null>(null)
  const [ccswitchOpen, setCcswitchOpen] = useState(false)
  // opencode 附加模式「从 opencode.json 导入」预览弹窗。
  const [opencodeImportOpen, setOpencodeImportOpen] = useState(false)
  const [query, setQuery] = useState("")
  // 切换确认对话框：切到缺必填项（端点/key/模板变量）的供应商前先问一句，
  // 用户确认后照切不误（「我已经知道它缺东西，就是要切」）。
  const [confirmSwitch, setConfirmSwitch] = useState<Provider | null>(null)
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

  /** 切换 app：重置 opencode 解释条（切走再切回重新提示）。 */
  function selectApp(a: App) {
    setApp(a)
    setBannerDismissed(false)
  }

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
                  // 激活态与侧栏 NavItem 选中同一套（tint 底 + 品牌字色）——单一
                  // 事实来源。去掉了原先叠加的底部 accent 色条：tint + 字色已够
                  // 区分，三重装饰显得绚丽。明暗由 --accent-tint /
                  // --accent-brand-strong 在 :root/.dark 各自定义，都成立。
                  "bg-accent-tint text-accent-brand-strong font-medium",
              )}
            >
              {t(`providers.app.${a}`)}
            </Button>
          ))}
        </fieldset>
        <div className="ml-auto flex gap-2">
          {/* 数据迁移类操作收进菜单：常规的导出 / 导入 / CC-Switch 导入在前，
              附加模式 (opencode) 特有的「从 opencode.json 导入」沉底——它是
              少数派路径，不挡常规入口。主操作「新增」独立为 primary。
              w-max: 菜单按内容宽度展开（共享组件默认菜单宽=触发器宽，而
              「从 opencode.json 导入」等长文案会把菜单撑得换行）。 */}
          <DropdownMenu>
            <DropdownMenuTrigger
              render={<Button variant="outline" size="sm" />}
            >
              <Database />
              {t("providers.dataMenu.label")}
              <ChevronDown />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-max">
              <DropdownMenuItem onClick={() => setTransfer("export")}>
                <Download />
                {t("providers.transfer.export")}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => setTransfer("import")}>
                <Upload />
                {t("providers.transfer.import")}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => setCcswitchOpen(true)}>
                <ArrowRightLeft />
                {t("providers.ccswitch.button")}
              </DropdownMenuItem>
              {isAdditive ? (
                <DropdownMenuItem onClick={() => setOpencodeImportOpen(true)}>
                  <RefreshCw />
                  {t("providers.live.import")}
                </DropdownMenuItem>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
          <Button size="sm" onClick={openNew}>
            <Plus />
            {t("providers.add")}
          </Button>
        </div>
      </div>

      {isAdditive && !bannerDismissed ? (
        // opencode 附加模式说明：多供应商共存，加入 live 即生效，无需切换。
        // 与 AppMark 描边方块呼应，让附加模式与单激活的差异在顶栏就可预期。
        <div className="bg-muted/40 flex items-center gap-2 rounded-lg px-3 py-2 text-xs">
          <Info className="text-muted-foreground size-3.5 shrink-0" />
          <span className="text-muted-foreground flex-1">
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
              <div className="text-muted-foreground grid grid-cols-[minmax(10rem,1.2fr)_8rem_minmax(0,1.4fr)_minmax(0,1fr)_10rem] gap-3 px-4 pb-2 text-xs">
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

      {/* 通用配置片段：按应用分派合并层（ADR-0010）——claude 在 settings_config 层
          并入、codex 在写盘层补缺失进 live 文件。卡片只对「切换时真正合并片段」
          的应用显示（#47 原则：不给配了不生效的应用展示控件）；gemini 的卡片随 #50
          （settings_config 层接线）、grok 的随 #51（写盘层接线）各自放开。opencode
          附加模式无「切换」概念，不显示。 */}
      {app === "claude" || app === "codex" ? (
        <CommonConfigSnippetCard app={app} />
      ) : null}

      <ProviderFormSheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        editing={editing}
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
      <CcSwitchImportDialog
        open={ccswitchOpen}
        onOpenChange={setCcswitchOpen}
      />
      <OpencodeImportDialog
        open={opencodeImportOpen}
        onOpenChange={setOpencodeImportOpen}
        app={app}
      />

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) {
            setDeleting(null)
            // Success leaves busy true (see onConfirmDelete) — release it on
            // every close path so the next dialog opens with a live button.
            setDeletingBusy(false)
          }
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
  // 缺必填项（端点 / key / 未物化模板变量）：提前到列表层可见，hover 列明细，
  // 不必等点「切换 / 加入 live」才在确认框里发现。official / cloud_provider 走
  // 默认端点或模板变量认证，不要求端点/key（见 providerMissingRequired）。
  const missingRequired = providerMissingRequired(p)
  return (
    <div
      ref={ref}
      className={cn(
        // 与列头模板一致（分类 8rem / 操作 10rem 固定）：行是独立 grid 容器，
        // 任何 auto 列都会让列宽逐行漂移——见列头注释。
        "grid grid-cols-[minmax(10rem,1.2fr)_8rem_minmax(0,1.4fr)_minmax(0,1fr)_10rem] items-center gap-3 rounded-lg px-4 py-2 transition-colors",
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
        {additive && liveManaged ? (
          /* 「已加入 live」状态徽标放名字旁而非操作区：操作列固定 10rem，
             徽标若留在按钮组会把操作列撑宽、列宽逐行漂移（见列头注释）。 */
          <Badge
            variant="outline"
            className="h-5 shrink-0 px-1.5 text-[11px] font-normal"
          >
            {t("providers.live.added")}
          </Badge>
        ) : null}
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
      {/* 行内按钮 hover 用 hover:!bg-accent-brand/25（主题色，比 accent-tint
          浓——accent-tint 只有品牌色 10-12% 透明，肉眼近于无）覆盖 Button
          默认的 hover:bg-muted：整行 hover 也是 bg-muted，按钮的 hover 反馈
          会被行吞掉（深浅主题都成立），! 确保覆盖。 */}
      <div className="flex justify-end gap-1">
        {additive ? (
          // 附加模式（opencode）：按 liveManaged 显示「移出」或「加入」，不走
          // 单激活的「切换 / 使用中」。已加入状态徽标在名字列（见上）。
          liveManaged ? (
            <Button
              variant="outline"
              size="sm"
              onClick={onRemoveFromLive}
              className="shrink-0 hover:!bg-accent-brand/25"
            >
              {t("providers.live.remove")}
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              onClick={onAddToLive}
              className="shrink-0 hover:!bg-accent-brand/25"
            >
              {t("providers.live.add")}
            </Button>
          )
        ) : isActive ? (
          <Badge
            variant="outline"
            className="h-6 gap-1 shrink-0 px-2 font-normal text-[11px]"
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
            className="shrink-0 hover:!bg-accent-brand/25"
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
