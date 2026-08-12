// Pending-upload confirmation. Each row's name is an editable input prefilled
// with the source basename. Status is recomputed live against the existing
// entries at the destination (and other rows): "new" or "same-name · overwrite".
// Same-name different-kind is not pre-detected here — the backend rejects it
// and surfaces a toast (rare; the drop flow asks the user to rename anyway).

import { Loader2 } from "lucide-react"
import { Fragment, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useAppInfoQuery,
  useScanLibraryQuery,
  useUploadToLibraryMutation,
} from "@/app/store/api"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import type { UploadItem } from "@/types/generated/bindings"
import { kindIcon } from "../kind-icon"

type Row = { sourcePath: string; name: string }

function basename(p: string): string {
  const norm = p.replace(/\\/g, "/").replace(/\/$/, "")
  const parts = norm.split("/")
  return parts[parts.length - 1] || p
}

/** The dialog only has dropped paths, not kinds — no extension is its best
 *  guess for "directory". Delegates to the shared kindIcon (single source). */
function rowIcon(name: string) {
  const ext = name.split(".").pop()?.toLowerCase()
  return kindIcon(name, !ext || name === ext)
}

export function UploadDialog({
  paths,
  subpath,
  onClose,
}: {
  paths: string[]
  subpath: string
  onClose: () => void
}) {
  const { t } = useTranslation()
  const [upload, { isLoading: busy }] = useUploadToLibraryMutation()
  const runWithToast = useMutateWithToast()
  const { data: info } = useAppInfoQuery()
  const selfId = info?.device_id ?? ""
  const { data: existing = [] } = useScanLibraryQuery({
    deviceScope: selfId,
    subpath,
  })

  const [rows, setRows] = useState<Row[]>(() =>
    paths.map((p) => ({ sourcePath: p, name: basename(p) })),
  )

  const existingNames = useMemo(
    () => new Set(existing.map((e) => e.name)),
    [existing],
  )

  async function onConfirm() {
    const items: UploadItem[] = rows.map((r) => ({
      source_path: r.sourcePath,
      target_name: r.name,
    }))
    const ok = await runWithToast(
      upload,
      { items, subpath },
      {
        success: {
          key: "library.toast.uploaded",
          vars: { count: items.length },
        },
        failed: { key: "library.toast.failed" },
      },
    )
    if (ok) onClose()
  }

  return (
    <Dialog open={true} onOpenChange={(o) => !o && onClose()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("library.upload.title")}</DialogTitle>
        </DialogHeader>
        <p className="text-muted-foreground text-xs">
          {t("library.upload.hint")}
        </p>
        <div className="flex flex-col gap-1.5">
          <div className="text-muted-foreground grid grid-cols-[1fr_9rem] gap-2 text-[11px]">
            <span className="flex items-center gap-2">
              {/* icon-width spacer so the label aligns with each row's input,
                not its leading icon. */}
              <span className="size-4 shrink-0" />
              {t("library.upload.col.name")}
            </span>
            <span className="justify-self-end">
              {t("library.upload.col.status")}
            </span>
          </div>
          {/* 大量文件时行区在对话框内滚动（max-h-64），列头留在滚动区外
            不跟着滚。滚动区与列头同构（grid-cols-[1fr_9rem]）：行距由
            gap-y 统一控制，行列与列头严格对齐。负 margin 让滚动条贴边。 */}
          <div className="max-h-64 -mr-1 grid grid-cols-[1fr_9rem] items-center gap-x-2 gap-y-1.5 overflow-y-auto pr-1">
            {rows.map((r, i) => {
              const Icon = rowIcon(r.name)
              const overwrite =
                existingNames.has(r.name) ||
                rows.some((other, j) => j !== i && other.name === r.name)
              return (
                <Fragment key={r.sourcePath}>
                  <div className="flex items-center gap-2">
                    <Icon className="text-muted-foreground size-4 shrink-0" />
                    <Input
                      value={r.name}
                      onChange={(e) =>
                        setRows((rs) =>
                          rs.map((x, j) =>
                            j === i ? { ...x, name: e.target.value } : x,
                          ),
                        )
                      }
                      className="h-7"
                    />
                  </div>
                  {overwrite ? (
                    <span className="text-[var(--sr-warn)] justify-self-end text-[11px] font-medium whitespace-nowrap">
                      {t("library.upload.status.overwrite")}
                    </span>
                  ) : (
                    <span className="text-muted-foreground justify-self-end text-[11px] whitespace-nowrap">
                      {t("library.upload.status.new")}
                    </span>
                  )}
                </Fragment>
              )
            })}
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t("library.upload.cancel")}
          </Button>
          <Button onClick={onConfirm} disabled={busy}>
            {busy ? <Loader2 className="animate-spin" /> : null}
            {busy
              ? t("library.upload.uploading")
              : t("library.upload.confirm", { count: rows.length })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
