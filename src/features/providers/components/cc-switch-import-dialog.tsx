// 「从 CC-Switch 导入」专用对话框：定位本机 CC-Switch 配置 → 翻译供应商 →
// 复用 apply_import 写库。不复用 ProviderTransferDialog（流程不同：无文件选择
// 步骤、要展示跳过明细报告）。自包含：内部调 importFromCcSwitch mutation、
// 展示报告（导入数 / 合并跳过 / 代理跳过名称列表）与「未找到 CC-Switch」错误。

import { AlertCircle, CheckCircle2, Loader2, Upload } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useImportFromCcSwitchMutation } from "@/app/store/api"
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
import { Label } from "@/components/ui/label"
import { rawErrorText } from "@/lib/error"
import type {
  App,
  CcSwitchImportReport,
  ProviderImportMode,
  SkipReason,
} from "@/types/generated/bindings"
import { IMPORT_MODE_OPTIONS, OptionCards } from "./option-cards"

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
  app,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** 当前视图应用——单应用语境，只导入该应用的供应商（ADR-0012）。 */
  app: App
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
    const result = await importCc({ app, mode, dbPath: dbPath.trim() || null })
    if (result.error) {
      // mutation 错误 → 人类可读消息（AppError 的 data 优先，rawErrorText 单一归属）。
      setError(rawErrorText(result.error))
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
          // 结果视图：成功 = emerald 色块（与 settings 的同步验证同一套
          // 「成功」语言）；跳过明细 = 分隔线列表，行内名字 + 原因徽标，
          // 标题不再复述原因（每项自标，原「跳过 1 个（需代理/OAuth 或不
          // 支持的应用）：」的括弧概括既绕又和明细重复）。
          <div className="flex flex-col gap-3">
            <div className="border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 flex items-start gap-2 rounded-md border p-2.5 text-sm">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" />
              <span>
                {t("providers.ccswitch.report.imported", {
                  count: report.imported,
                })}
              </span>
            </div>
            {report.mergeSkipped > 0 ? (
              <p className="text-muted-foreground text-xs">
                {t("providers.ccswitch.report.mergeSkipped", {
                  count: report.mergeSkipped,
                })}
              </p>
            ) : null}
            {report.proxySkipped.length > 0 ? (
              <div className="flex flex-col">
                <p className="text-muted-foreground mb-1.5 text-xs font-medium">
                  {t("providers.ccswitch.report.skipped", {
                    count: report.proxySkipped.length,
                  })}
                </p>
                {report.proxySkipped.map((s) => (
                  /* Each skipped provider appears once per report, so the
                     name is a stable key (no index suffix needed). */
                  <div
                    key={s.name}
                    className="flex items-center justify-between gap-3 border-b border-border py-1.5 text-sm last:border-b-0"
                  >
                    <span className="truncate">{s.name}</span>
                    <Badge variant="secondary" className="shrink-0 font-normal">
                      {t(SKIP_REASON_LABEL[s.reason])}
                    </Badge>
                  </div>
                ))}
              </div>
            ) : null}
          </div>
        ) : (
          // 表单视图：冲突策略 + 可选配置位置。
          // 冲突策略用可见决策卡（OptionCards，与 JSON 迁移弹窗共享）：
          // 选项平铺可对比、语义在卡片内——下拉藏选项 + 行下动态小字在切换
          // 时会文字跳变撑高布局，正是这个弹窗「乱」的来源。
          <div className="flex flex-col gap-4">
            <div className="flex flex-col gap-2">
              <Label>{t("providers.transfer.mode.label")}</Label>
              <OptionCards
                value={mode}
                onValueChange={setMode}
                options={IMPORT_MODE_OPTIONS}
                ariaLabel={t("providers.transfer.mode.label")}
              />
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
              {/* placeholder 示范输入格式，hint 承诺留空自动检测——各司其职，
                  不再像原来的「自动检测（留空）」占位符那样和 hint 重复。 */}
              <p className="text-muted-foreground text-xs">
                {t("providers.ccswitch.pathHint")}
              </p>
            </div>
            {error ? (
              // 错误用色块 + 图标与上面的 hint 区分，一眼可辨是出错了。
              <p className="bg-destructive/10 text-destructive flex items-start gap-1.5 rounded-md px-3 py-2 text-xs">
                <AlertCircle className="mt-0.5 size-3.5 shrink-0" />
                <span>{error}</span>
              </p>
            ) : null}
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
