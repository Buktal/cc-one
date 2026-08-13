// 「导入」入口对话框：三个导入来源收进同一个入口，点卡片即进入对应流程——
// 从本机配置文件导入 / 从 CC-Switch 迁移（外部来源，ADR-0012）/ 从 CC One
// 备份恢复，三个平级排列。选择即导航：本对话框不持状态，点击后关闭并把流程
// 交给调用方打开的对应对话框（LiveImportDialog / CcSwitchImportDialog /
// ProviderTransferDialog）。
//
// 之所以做「来源选择」而非下拉菜单：导入入口需要一眼可见有哪些来源，用户
// 不必点开才知道里面有 CC-Switch 导入。

import { ChevronRight, Database, FileJson, RefreshCw } from "lucide-react"
import { useTranslation } from "react-i18next"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { cn } from "@/lib/utils"

export function ImportSourceDialog({
  open,
  onOpenChange,
  onImportCcSwitch,
  onImportLive,
  onImportBackup,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onImportCcSwitch: () => void
  onImportLive: () => void
  onImportBackup: () => void
}) {
  const { t } = useTranslation()
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("providers.transfer.importTitle")}</DialogTitle>
          <DialogDescription>
            {t("providers.importSource.hint")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-2">
          <SourceCard
            icon={<FileJson className="size-4" />}
            title={t("providers.live.import")}
            description={t("providers.liveImport.hint")}
            onClick={() => {
              onOpenChange(false)
              onImportLive()
            }}
          />
          <SourceCard
            icon={<RefreshCw className="size-4" />}
            title={t("providers.ccswitch.title")}
            description={t("providers.ccswitch.hint")}
            onClick={() => {
              onOpenChange(false)
              onImportCcSwitch()
            }}
          />
          <SourceCard
            icon={<Database className="size-4" />}
            title={t("providers.importMenu.backup")}
            description={t("providers.transfer.importHint")}
            onClick={() => {
              onOpenChange(false)
              onImportBackup()
            }}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}

/** 来源卡片：整行可点（选择即行动），图标方块 + 标题/描述 + 右箭头。 */
function SourceCard({
  icon,
  title,
  description,
  onClick,
}: {
  icon: React.ReactNode
  title: string
  description: string
  onClick: () => void
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "group flex w-full items-center gap-3 rounded-lg border p-3 text-left",
        "transition-colors hover:bg-hover",
        "focus-visible:ring-ring/40 focus-visible:outline-none focus-visible:ring-2",
      )}
    >
      <span className="bg-muted text-muted-foreground flex size-9 shrink-0 items-center justify-center rounded-md">
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-muted-foreground text-xs leading-relaxed">
          {description}
        </span>
      </span>
      <ChevronRight className="text-muted-foreground size-4 shrink-0 transition-transform group-hover:translate-x-0.5" />
    </button>
  )
}
