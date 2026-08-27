// Pricing entry editor as a modal Dialog (was an inline panel that pushed the
// table down). New and edit both flow through here; on save it calls the
// mutation and closes. User-edited entries are marked is_builtin=false so a
// later LiteLLM pull won't clobber them.
//
// Display: the four prices form a 2×2 matrix under a SectionHeader that states
// the shared unit (USD / 1M tokens) once, so cell labels stay short. Each cell
// is a tabular-num input with a $ prefix, and a 0 price shows a 免费 pill —
// free lanes scan at a glance. Prices are kept as strings while typing (a
// controlled number input snaps back to 0 mid-scientific-notation like `1e`)
// and parsed on save via parsePriceInput. The model key input is focused on
// open (initialFocus) and the whole form submits on Enter.

import { Info } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useSavePricingMutation } from "@/app/store/api"
import { Field } from "@/components/form-field"
import { SectionHeader } from "@/components/section-header"
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
import { parsePriceInput } from "@/features/pricing/derive"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { cn } from "@/lib/utils"

import type { PricingEntry } from "@/types/generated/bindings"

export function emptyEntry(): PricingEntry {
  return {
    model_key: "",
    display_name: "",
    input_per_million: 0,
    output_per_million: 0,
    cache_read_per_million: 0,
    cache_creation_per_million: 0,
    is_builtin: false,
  }
}

type PriceKey = "input" | "output" | "cacheRead" | "cacheCreation"

interface EntryDraft {
  model_key: string
  display_name: string
  prices: Record<PriceKey, string>
}

/** PricingEntry → string draft; prices stay strings until save (see header). */
function toDraft(entry: PricingEntry | null): EntryDraft {
  return {
    model_key: entry?.model_key ?? "",
    display_name: entry?.display_name ?? "",
    prices: {
      input: String(entry?.input_per_million ?? 0),
      output: String(entry?.output_per_million ?? 0),
      cacheRead: String(entry?.cache_read_per_million ?? 0),
      cacheCreation: String(entry?.cache_creation_per_million ?? 0),
    },
  }
}

export function EntryEditorDialog({
  open,
  onOpenChange,
  entry,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  entry: PricingEntry | null
  onSaved: () => void
}) {
  const { t } = useTranslation()
  const keyRef = useRef<HTMLInputElement>(null)
  const [draft, setDraft] = useState<EntryDraft>(() => toDraft(entry))
  const [save, { isLoading: saving }] = useSavePricingMutation()
  const runWithToast = useMutateWithToast()

  useEffect(() => {
    if (open) setDraft(toDraft(entry))
  }, [entry, open])

  const setPrice = (k: PriceKey) => (v: string) =>
    setDraft((d) => ({ ...d, prices: { ...d.prices, [k]: v } }))

  async function onSave() {
    const modelKey = draft.model_key.trim()
    if (!modelKey) {
      toast.error(t("pricing.toast.modelKeyRequired"))
      return
    }
    const ok = await runWithToast(
      save,
      // 保存即视为用户自定义：编辑内置条目后也要落成自定义（is_builtin=false），
      // 否则下次 LiteLLM 拉取会无条件覆盖。Rust 侧 unwrap_or(false) 兜底。
      {
        entry: {
          model_key: modelKey,
          display_name: draft.display_name.trim(),
          input_per_million: parsePriceInput(draft.prices.input),
          output_per_million: parsePriceInput(draft.prices.output),
          cache_read_per_million: parsePriceInput(draft.prices.cacheRead),
          cache_creation_per_million: parsePriceInput(
            draft.prices.cacheCreation,
          ),
          is_builtin: false,
        },
        isBuiltin: false,
      },
      {
        success: {
          key: "pricing.toast.saved",
          vars: { key: modelKey },
        },
        failed: { key: "settings.toast.saveFailed" },
      },
    )
    if (ok) onSaved()
  }

  const priceFields: { key: PriceKey; label: string }[] = [
    { key: "input", label: t("pricing.editor.input") },
    { key: "output", label: t("pricing.editor.output") },
    { key: "cacheRead", label: t("pricing.editor.cacheRead") },
    { key: "cacheCreation", label: t("pricing.editor.cacheCreation") },
  ]

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent initialFocus={keyRef}>
        <DialogHeader>
          <DialogTitle>
            {entry?.model_key
              ? t("pricing.editor.editTitle")
              : t("pricing.editor.newTitle")}
          </DialogTitle>
          <DialogDescription>
            {t("pricing.editor.description")}
          </DialogDescription>
        </DialogHeader>

        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault()
            void onSave()
          }}
        >
          <div className="grid grid-cols-2 gap-3">
            <Field label={t("pricing.col.modelKey")}>
              {/* 模型标识是表格里的主键（表格用 mono 渲染）——输入框同样用 mono
                字体，保持「标识」的身份感；打开弹窗即聚焦，新增时直接可输入。 */}
              <Input
                ref={keyRef}
                value={draft.model_key}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, model_key: e.target.value }))
                }
                placeholder={t("pricing.editor.modelKeyPlaceholder")}
                className="font-mono"
              />
            </Field>
            <Field label={t("pricing.col.displayName")}>
              <Input
                value={draft.display_name}
                onChange={(e) =>
                  setDraft((d) => ({ ...d, display_name: e.target.value }))
                }
              />
            </Field>

            {/* 单位只出现在分节标题里一次（表格把 `$/1M` 放表头同理），四个
              单元格标签保持短名。 */}
            <SectionHeader className="col-span-2">
              {t("pricing.editor.priceSection")}
            </SectionHeader>
            {priceFields.map(({ key, label }) => (
              <PriceField
                key={key}
                label={label}
                value={draft.prices[key]}
                onChange={setPrice(key)}
                freeLabel={t("pricing.editor.free")}
              />
            ))}
          </div>

          {/* 编辑内置条目时预告状态变化（独立成行，不再塞进描述文字里）：
            保存后即变自定义，LiteLLM 拉取不再覆盖。 */}
          {entry?.is_builtin ? (
            <div className="text-muted-foreground border-border/60 flex items-start gap-2 rounded-md border bg-muted/40 px-3 py-2 text-xs">
              <Info className="mt-0.5 size-3.5 shrink-0" />
              {t("pricing.editor.builtinNotice")}
            </div>
          ) : null}

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
            >
              {t("common.cancel")}
            </Button>
            <Button type="submit" disabled={saving}>
              {saving ? t("common.saving") : t("common.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** 单个价格单元格：$ 前缀 + 等宽 tabular 数字 + 0 价时的「免费」pill。
 *  （Field 原子已下放 @/components/form-field，本地抄本删除——架构审查
 *  Ⅲ候选⑫。） */
function PriceField({
  label,
  value,
  onChange,
  freeLabel,
}: {
  label: string
  value: string
  onChange: (v: string) => void
  freeLabel: string
}) {
  const isFree = parsePriceInput(value) === 0
  return (
    <div className="flex flex-col gap-1.5">
      <Label className="text-muted-foreground text-xs">{label}</Label>
      <div className="relative">
        <span
          aria-hidden
          className="text-muted-foreground pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-sm"
        >
          $
        </span>
        <Input
          type="number"
          min="0"
          step="0.0001"
          inputMode="decimal"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className={cn("pr-9 pl-6 tabular-nums", isFree && "pr-10")}
        />
        {isFree ? (
          <span
            aria-hidden
            className="text-muted-foreground pointer-events-none absolute top-1/2 right-1.5 -translate-y-1/2 rounded-sm bg-muted px-1 text-[10px] leading-4"
          >
            {freeLabel}
          </span>
        ) : null}
      </div>
    </div>
  )
}
