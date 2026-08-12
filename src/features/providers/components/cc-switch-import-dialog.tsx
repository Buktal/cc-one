// 「从 CC-Switch 导入」专用对话框：定位本机 CC-Switch 配置 → 翻译供应商 →
// 复用 apply_import 写库。不复用 ProviderTransferDialog（流程不同：无文件选择
// 步骤、要展示跳过明细报告）。自包含：内部调 importFromCcSwitch mutation、
// 展示报告（导入数 / 合并跳过 / 代理跳过名称列表）与「未找到 CC-Switch」错误。

import { Loader2, Upload } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useImportFromCcSwitchMutation } from "@/app/store/api"
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
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { toStructuredError } from "@/lib/error"
import type {
  CcSwitchImportReport,
  ProviderImportMode,
  SkipReason,
} from "@/types/generated/bindings"

const MODE_OPTIONS: ReadonlyArray<[ProviderImportMode, string]> = [
  ["merge", "providers.transfer.mode.merge"],
  ["overwrite", "providers.transfer.mode.overwrite"],
]

/** 跳过原因 → i18n key。枚举变体跨边界转成 snake_case（needs_proxy /
 *  needs_o_auth / unsupported_app——注意 OAuth 被拆成 o_auth，与后端 serde 一致）。 */
const SKIP_REASON_LABEL: Record<SkipReason, string> = {
  needs_proxy: "providers.ccswitch.skipReason.needsProxy",
  needs_o_auth: "providers.ccswitch.skipReason.needsOAuth",
  unsupported_app: "providers.ccswitch.skipReason.unsupportedApp",
}

export function CcSwitchImportDialog({
  open,
  onOpenChange,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()
  const [mode, setMode] = useState<ProviderImportMode>("merge")
  const [dbPath, setDbPath] = useState("")
  const [importCc, { isLoading }] = useImportFromCcSwitchMutation()
  const [report, setReport] = useState<CcSwitchImportReport | null>(null)
  const [error, setError] = useState<string | null>(null)

  function close() {
    onOpenChange(false)
    // 关闭时清状态，下次打开是干净表单。
    setReport(null)
    setError(null)
    setDbPath("")
    setMode("merge")
  }

  async function onConfirm() {
    setError(null)
    setReport(null)
    const result = await importCc({ mode, dbPath: dbPath.trim() || null })
    if (result.error) {
      const structured = toStructuredError(result.error)
      setError(
        structured?.kind === "app"
          ? structured.data
          : (structured?.message ?? String(result.error)),
      )
      return
    }
    setReport(result.data)
  }

  if (!open) return null

  return (
    <Dialog open onOpenChange={(o) => !o && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("providers.ccswitch.title")}</DialogTitle>
          <DialogDescription>{t("providers.ccswitch.hint")}</DialogDescription>
        </DialogHeader>

        {report ? (
          // 结果视图：导入数 / 合并跳过 / 代理跳过明细。
          <div className="flex flex-col gap-3 text-sm">
            <p>{t("providers.ccswitch.report.imported", { count: report.imported })}</p>
            {report.mergeSkipped > 0 ? (
              <p className="text-muted-foreground">
                {t("providers.ccswitch.report.mergeSkipped", {
                  count: report.mergeSkipped,
                })}
              </p>
            ) : null}
            {report.proxySkipped.length > 0 ? (
              <div className="flex flex-col gap-1">
                <p className="text-muted-foreground">
                  {t("providers.ccswitch.report.skipped", {
                    count: report.proxySkipped.length,
                  })}
                </p>
                <ul className="text-muted-foreground list-disc pl-5 text-xs">
                  {report.proxySkipped.map((s, i) => (
                    <li key={`${s.name}-${i}`}>
                      {s.name}（{t(SKIP_REASON_LABEL[s.reason])}）
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        ) : (
          // 表单视图：冲突模式 + 可选配置位置。
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-2">
              <Label>{t("providers.transfer.mode.label")}</Label>
              <Select
                value={mode}
                onValueChange={(v) => setMode(v as ProviderImportMode)}
              >
                <SelectTrigger className="w-64">
                  <SelectValue>
                    {(v: string) =>
                      t(MODE_OPTIONS.find((o) => o[0] === v)?.[1] ?? v)
                    }
                  </SelectValue>
                </SelectTrigger>
                <SelectContent>
                  {MODE_OPTIONS.map(([value, key]) => (
                    <SelectItem key={value} value={value}>
                      {t(key)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-muted-foreground text-xs">
                {mode === "merge"
                  ? t("providers.transfer.mode.mergeHint")
                  : t("providers.transfer.mode.overwriteHint")}
              </p>
            </div>
            <div className="flex flex-col gap-2">
              <Label>{t("providers.ccswitch.path")}</Label>
              <Input
                value={dbPath}
                onChange={(e) => setDbPath(e.target.value)}
                placeholder={t("providers.ccswitch.pathPlaceholder")}
                spellCheck={false}
                className="font-mono text-xs"
              />
              <p className="text-muted-foreground text-xs">
                {t("providers.ccswitch.pathHint")}
              </p>
            </div>
            {error ? <p className="text-destructive text-xs">{error}</p> : null}
          </div>
        )}

        <DialogFooter>
          {report ? (
            <Button onClick={close}>{t("common.close")}</Button>
          ) : (
            <>
              <Button variant="outline" onClick={close}>
                {t("common.cancel")}
              </Button>
              <Button onClick={onConfirm} disabled={isLoading}>
                {isLoading ? <Loader2 className="animate-spin" /> : <Upload />}
                {t("providers.ccswitch.import")}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
