// 多设备列表。列出所有已知设备；本机由 is_self 标记；可给其他设备起显示名
// （setDeviceDisplayName），也可「删除」对端设备——本地遗忘：移除其本机注册
// 行 + 用量历史 + 本地产物目录，不推 git；若该设备仍在别处活跃，下次同步会
// 自动回来。本机不可删（只能改名）。
//
// Content-only — 渲染在 SettingsView 的「设备」分区卡片内，不再自带 Card 壳。

import { Trash2 } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"

import {
  useDevicesQuery,
  useForgetDeviceMutation,
  useLibraryDeviceSummaryQuery,
  useSetDeviceDisplayNameMutation,
} from "@/app/store/api"
import { InlineTextEdit } from "@/components/inline-text-edit"
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
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useInlineEdit } from "@/hooks/use-inline-edit"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { cn } from "@/lib/utils"
import type {
  DeviceInfo,
  LibraryForgetAction,
} from "@/types/generated/bindings"

/** The two choices shown when a device has library files. Order = UI order;
 *  the first (migrate) is the default selection. */
const FORGET_CHOICES: Array<{
  id: LibraryForgetAction
  titleKey: string
  hintKey: string
}> = [
  {
    id: "migrate",
    titleKey: "devices.forget.choice.migrate",
    hintKey: "devices.forget.choice.migrateHint",
  },
  {
    id: "delete",
    titleKey: "devices.forget.choice.delete",
    hintKey: "devices.forget.choice.deleteHint",
  },
]

export function DeviceList() {
  const { t } = useTranslation()
  const { data: devices = [] } = useDevicesQuery()
  const [setName] = useSetDeviceDisplayNameMutation()
  const [forget, { isLoading: forgetting }] = useForgetDeviceMutation()
  const runWithToast = useMutateWithToast()
  // 设备改名编辑器：draft/open/busy 机器归 useInlineEdit，呈现归
  // InlineTextEdit（✓/✕ 内嵌输入框；Enter 提交 / Escape / 失焦收起的契约
  // 在组件里）。target 存 begin 时抓的设备对象，按 device_id 比对。
  const rename = useInlineEdit<DeviceInfo>({
    commit: (device, draft) =>
      runWithToast(
        setName,
        { deviceId: device.device_id, displayName: draft.trim() },
        {
          success: { key: "settings.toast.displayNameUpdated" },
          failed: { key: "settings.toast.renameFailed" },
        },
      ),
  })
  const [removing, setRemoving] = useState<string | null>(null)
  const [libAction, setLibAction] = useState<LibraryForgetAction>("migrate")
  // Pre-flight: does this peer have library files? Drives the migrate-vs-delete
  // choice. Skipped while no device is pending removal.
  const { data: summary } = useLibraryDeviceSummaryQuery(removing ?? "", {
    skip: removing === null,
  })
  const hasLibrary = (summary?.files ?? 0) > 0 || (summary?.dirs ?? 0) > 0

  if (devices.length === 0) {
    return (
      <span className="text-muted-foreground text-sm">
        {t("devices.empty")}
      </span>
    )
  }

  const target = devices.find((d) => d.device_id === removing)
  const targetName = target ? target.display_name || t("common.unnamed") : ""

  async function onConfirmForget() {
    if (!removing) return
    const id = removing
    const name = targetName
    const action = libAction
    const ok = await runWithToast(
      forget,
      { deviceId: id, libraryAction: action },
      {
        success: { key: "devices.removed", vars: { name } },
        failed: { key: "devices.removeFailed" },
      },
    )
    if (ok) setRemoving(null)
  }

  return (
    <>
      <div className="flex flex-col">
        {devices.map((d, i) => (
          <div
            key={d.device_id}
            className={`flex items-center justify-between gap-3 py-2 ${i === devices.length - 1 ? "" : "border-b"}`}
          >
            <div className="flex min-w-0 flex-col gap-0.5">
              <span className="flex items-center gap-2">
                <span className="truncate font-medium">
                  {d.display_name || t("common.unnamed")}
                </span>
                {d.is_self ? (
                  <Badge variant="secondary">{t("devices.thisDevice")}</Badge>
                ) : null}
              </span>
              <span className="text-muted-foreground truncate font-mono text-xs">
                {d.device_id}
              </span>
            </div>
            {d.is_self ? null : rename.target?.device_id === d.device_id ? (
              <InlineTextEdit
                className="w-44"
                value={rename.draft}
                onValueChange={rename.setDraft}
                busy={rename.busy}
                onCommit={() => void rename.commit()}
                onCancel={rename.cancel}
                placeholder={t("devices.displayNamePlaceholder")}
                ariaLabel={t("devices.rename")}
                autoFocus
              />
            ) : (
              <div className="flex items-center gap-1">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => rename.begin(d, d.display_name ?? "")}
                >
                  {t("devices.rename")}
                </Button>
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        aria-label={t("devices.remove")}
                        onClick={() => {
                          setLibAction("migrate")
                          setRemoving(d.device_id)
                        }}
                      />
                    }
                  >
                    <Trash2 className="text-muted-foreground" />
                  </TooltipTrigger>
                  <TooltipContent className="max-w-56 text-center">
                    {t("devices.removeTooltip")}
                  </TooltipContent>
                </Tooltip>
              </div>
            )}
          </div>
        ))}
      </div>

      <Dialog
        open={removing !== null}
        onOpenChange={(o) => {
          if (!o) setRemoving(null)
        }}
      >
        <DialogContent showClose={false} className="max-w-md">
          <DialogHeader>
            <DialogTitle>{t("devices.removeConfirmTitle")}</DialogTitle>
            <DialogDescription>
              {hasLibrary
                ? t("devices.forget.hasLibraryPrompt", {
                    files: summary?.files ?? 0,
                    dirs: summary?.dirs ?? 0,
                  })
                : t("devices.removeWarning", { name: targetName })}
            </DialogDescription>
          </DialogHeader>
          {hasLibrary ? (
            <div className="flex flex-col gap-2">
              {FORGET_CHOICES.map((choice) => {
                const selected = libAction === choice.id
                return (
                  <button
                    key={choice.id}
                    type="button"
                    onClick={() => setLibAction(choice.id)}
                    className={cn(
                      "flex items-start gap-2.5 rounded-md border p-3 text-left text-sm transition-colors",
                      selected
                        ? "border-accent-brand-strong bg-accent-tint"
                        : "border-border hover:bg-hover",
                    )}
                  >
                    <span
                      className={cn(
                        "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border",
                        selected
                          ? "border-accent-brand-strong"
                          : "border-muted-foreground/40",
                      )}
                    >
                      {selected ? (
                        <span className="bg-accent-brand-strong size-2 rounded-full" />
                      ) : null}
                    </span>
                    <span className="flex min-w-0 flex-col gap-0.5">
                      <span className="font-medium">{t(choice.titleKey)}</span>
                      <span className="text-muted-foreground text-xs">
                        {t(choice.hintKey, { name: targetName })}
                      </span>
                    </span>
                  </button>
                )
              })}
            </div>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setRemoving(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="destructive"
              disabled={forgetting}
              onClick={onConfirmForget}
            >
              {forgetting ? t("common.removing") : t("devices.remove")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
