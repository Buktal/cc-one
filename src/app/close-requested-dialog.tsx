// Close-requested dialog. When the main window's close is
// intercepted with close_behavior = Ask, Rust emits `close-requested`; this
// component shows the minimize/quit dialog and resolves it via `confirmClose`.

import { listen } from "@tauri-apps/api/event"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { confirmClose } from "@/app/store/api"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"

export function CloseRequestedDialog() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [remember, setRemember] = useState(false)

  useEffect(() => {
    const off = listen("close-requested", () => setOpen(true))
    return () => {
      off.then((unlisten) => unlisten())
    }
  }, [])

  const choose = (behavior: "minimize" | "quit") => {
    setOpen(false)
    void confirmClose(behavior, remember)
    setRemember(false)
  }

  return (
    <Dialog open={open} onOpenChange={(o) => setOpen(o)}>
      <DialogContent showClose={false}>
        <DialogHeader>
          <DialogTitle>{t("closeDialog.title")}</DialogTitle>
          <DialogDescription>{t("closeDialog.description")}</DialogDescription>
        </DialogHeader>
        <label
          htmlFor="close-remember"
          className="flex cursor-pointer items-center gap-2 text-sm"
        >
          <Checkbox
            id="close-remember"
            checked={remember}
            onCheckedChange={(v) => setRemember(v === true)}
          />
          {t("closeDialog.remember")}
        </label>
        <DialogFooter>
          <Button variant="outline" onClick={() => choose("quit")}>
            {t("common.quit")}
          </Button>
          <Button onClick={() => choose("minimize")}>
            {t("closeDialog.minimizeToTray")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
