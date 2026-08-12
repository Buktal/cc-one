// Export / import dialogs for provider configs — manual migration (换设备迁移 /
// 留档), deliberately NOT a git-sync path: export writes a JSON document to a
// user-chosen path, import applies one back to the local DB only.
//
// Thin UI shell: the file dialogs, mutations and toasts live in
// `useProvidersBrowser` (same split as the library browser). This component
// only holds the option state (include keys? conflict mode?) and the confirm
// buttons, and reports success back so the caller closes the dialog.

import { Download, Loader2, Upload } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import type { ProviderImportMode } from "@/types/generated/bindings"
import {
  IMPORT_MODE_OPTIONS,
  type OptionCardOption,
  OptionCards,
} from "./option-cards"

/** Which transfer direction the dialog is showing. */
export type TransferKind = "export" | "import"

/** Export key option: strip secrets (default) or carry them for migration. */
type KeysChoice = "withoutKeys" | "withKeys"

/** 导出密钥选项：与导入冲突处理同一套决策卡视觉（OptionCards），
 *  语义写进卡片内部。 */
const KEYS_OPTIONS: ReadonlyArray<OptionCardOption<KeysChoice>> = [
  {
    value: "withoutKeys",
    labelKey: "providers.transfer.keys.withoutKeys",
    hintKey: "providers.transfer.keys.withoutKeysHint",
  },
  {
    value: "withKeys",
    labelKey: "providers.transfer.keys.withKeys",
    hintKey: "providers.transfer.keys.withKeysHint",
  },
]

export function ProviderTransferDialog({
  kind,
  transferring,
  onExport,
  onImport,
  onOpenChange,
}: {
  kind: TransferKind | null
  transferring: boolean
  onExport: (includeKeys: boolean) => Promise<boolean>
  onImport: (mode: ProviderImportMode) => Promise<boolean>
  onOpenChange: (kind: TransferKind | null) => void
}) {
  const { t } = useTranslation()
  const [keysChoice, setKeysChoice] = useState<KeysChoice>("withoutKeys")
  const [mode, setMode] = useState<ProviderImportMode>("merge")

  if (!kind) return null
  const isExport = kind === "export"

  async function onConfirm() {
    const ok = isExport
      ? await onExport(keysChoice === "withKeys")
      : await onImport(mode)
    if (ok) onOpenChange(null)
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onOpenChange(null)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {isExport
              ? t("providers.transfer.exportTitle")
              : t("providers.transfer.importTitle")}
          </DialogTitle>
          <DialogDescription>
            {isExport
              ? t("providers.transfer.exportHint")
              : t("providers.transfer.importHint")}
          </DialogDescription>
        </DialogHeader>
        {isExport ? (
          <div className="flex flex-col gap-2">
            <Label>{t("providers.transfer.keys.label")}</Label>
            {/* 与导入冲突处理同一套决策卡：选项平铺、语义进卡片。 */}
            <OptionCards
              value={keysChoice}
              onValueChange={setKeysChoice}
              options={KEYS_OPTIONS}
              ariaLabel={t("providers.transfer.keys.label")}
            />
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <Label>{t("providers.transfer.mode.label")}</Label>
            {/* 与 CC-Switch 导入弹窗共享同一决策卡：选项平铺、语义进卡片。 */}
            <OptionCards
              value={mode}
              onValueChange={setMode}
              options={IMPORT_MODE_OPTIONS}
              ariaLabel={t("providers.transfer.mode.label")}
            />
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(null)}>
            {t("common.cancel")}
          </Button>
          <Button onClick={onConfirm} disabled={transferring}>
            {transferring ? (
              <Loader2 className="animate-spin" />
            ) : isExport ? (
              <Download />
            ) : (
              <Upload />
            )}
            {isExport
              ? t("providers.transfer.export")
              : t("providers.transfer.import")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
