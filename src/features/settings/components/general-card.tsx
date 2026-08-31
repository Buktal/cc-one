// General preferences: display language,
// background-collect interval, push-to-sync interval, window-close behavior.
// Content-only — rendered inside SettingsView's 通用
// section card, so no Card wrapper of its own.
//
// Collect and push are DECOUPLED: collect is a short seconds-level
// local cadence, push is a longer minutes-level Git cadence (Synced only).
// Language is the one preference Rust must know at cold start (to
// build the localized tray), so it lives here alongside the others. All
// discrete presets (Select, instant-effect) — no save button.
//
// 两栏行网格 (#109, 决议 #99 variant-a): 视口 ≥1240px 时卡内分「界面」/
// 「运行」两栏（竖分隔线 + 眉题），以下退回单栏行布局（窄窗行为与现状
// 一致）。Row-based layout: each preference is a SettingRow — label +
// hint on the left, control on the right, hairline between rows — so the card
// stays scannable as more options land. 秒/分级预设三行（autoTuck /
// collect / push）塌缩为 PresetSelect + 常量表：触发器与选项列表共用同一个
// formatLabel（内走 formatDurationLabel 的秒/分/时分档），档位文案只写
// 一次——选项显示与触发器显示不可能再分裂（曾内联 4 份「0=off / <60 秒 /
// 否则分钟」，加小时档要改 4 个点）。无 render 函数时 Base UI 显示裸值
// ("10" / "300" / "zh")，不是本地化的 "10 秒" / "5 分钟" / "中文"。
// 版本与更新行移至「关于」区（#109 承接自 shell）。

import { Check } from "lucide-react"
import { useTranslation } from "react-i18next"
import {
  useAppInfoQuery,
  usePreferencesQuery,
  useSetCloseBehaviorMutation,
  useSetCollectIntervalMutation,
  useSetLanguageMutation,
  useSetLightweightAutoTuckMutation,
  useSetLightweightExpandMutation,
  useSetPushIntervalMutation,
  useSetSkinMutation,
} from "@/app/store/api"
import { InlineBanner } from "@/components/inline-banner"
import { Button } from "@/components/ui/button"
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
import { SettingRow } from "@/features/settings/components/setting-row"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { LANGUAGES } from "@/i18n/languages"
import { describeError } from "@/lib/error"
import { formatDurationLabel } from "@/lib/format"
import { cn } from "@/lib/utils"
import type {
  CloseBehavior,
  Language,
  LightweightExpand,
  Skin_Serialize,
} from "@/types/generated/bindings"

/** Close-behavior option → i18n key. */
const CLOSE_OPTIONS: ReadonlyArray<[CloseBehavior, string]> = [
  ["ask", "settings.general.closeAsk"],
  ["minimize", "closeDialog.minimizeToTray"],
  ["quit", "common.quit"],
]

/** Lightweight half-icon expand trigger: click (default) or hover. */
const EXPAND_OPTIONS: ReadonlyArray<[LightweightExpand, string]> = [
  ["click", "settings.general.lightweightExpandClick"],
  ["hover", "settings.general.lightweightExpandHover"],
]

/** Auto-tuck presets: delay before an invisible full window morphs into the
 *  mini bar; 0 = off. Seconds (matching the collect/push preset pattern). */
const AUTO_TUCK_OPTIONS: ReadonlyArray<number> = [0, 10, 30, 60, 300]

/** Collect presets: seconds-level, local-only. */
const COLLECT_OPTIONS: ReadonlyArray<number> = [5, 10, 30, 60]

/** Push presets: minutes-level, Git, Synced only. */
const PUSH_OPTIONS: ReadonlyArray<number> = [300, 600, 900, 1800, 3600]

/**
 * Color skins (multi-skin theming). Chromatic swatches read straight from CSS
 * — each carries `data-skin={value}` and uses `var(--brand)`, so the swatch IS
 * the live accent from index.css (single source: edit a [data-skin] block and
 * the swatch follows, no TS sync). `neutral` is the exception: its grey is the
 * :root/.dark default with NO [data-skin] block, so var(--brand) would inherit
 * the active skin's brand on <html> — it uses a literal `brand` fill instead.
 * The selection check follows the MODE, not the skin: black in light, white in
 * dark, with a dark drop-shadow so it reads on any swatch fill. Names are
 * English literals (no i18n); `neutral` first as the default.
 */
const SKINS: ReadonlyArray<{
  value: Skin_Serialize
  label: string
  brand?: string
}> = [
  {
    value: "neutral",
    label: "Neutral",
    brand: "#6b7280",
  },
  { value: "sage", label: "Sage" },
  { value: "azure", label: "Azure" },
  { value: "crimson", label: "Crimson" },
  { value: "mauve", label: "Mauve" },
]

export function GeneralCard() {
  const { t } = useTranslation()
  const { data: prefs, error: prefsError } = usePreferencesQuery()
  const { data: info } = useAppInfoQuery()
  const synced = info?.mode === "synced"
  const [setLanguage, { isLoading: savingLang }] = useSetLanguageMutation()
  const [setLightweightExpand, { isLoading: savingExpand }] =
    useSetLightweightExpandMutation()
  const [setAutoTuck, { isLoading: savingAutoTuck }] =
    useSetLightweightAutoTuckMutation()
  const [setCloseBehavior, { isLoading: savingClose }] =
    useSetCloseBehaviorMutation()
  const [setCollectInterval, { isLoading: savingCollect }] =
    useSetCollectIntervalMutation()
  const [setPushInterval, { isLoading: savingPush }] =
    useSetPushIntervalMutation()
  const [setSkin, { isLoading: savingSkin }] = useSetSkinMutation()
  const runWithToast = useMutateWithToast()

  return (
    <div className="flex flex-col">
      {prefsError ? (
        <InlineBanner tone="error" className="mb-2">
          {t("settings.general.readError", {
            detail: describeError(prefsError, t) || t("common.unknownReason"),
          })}
        </InlineBanner>
      ) : null}

      <div className="grid min-[1240px]:grid-cols-2 min-[1240px]:gap-0">
        {/* 界面 — presentation preferences */}
        <div className="flex flex-col min-[1240px]:pr-8">
          <GroupCap>{t("settings.general.groupInterface")}</GroupCap>
          <SettingRow
            label={t("settings.general.skin")}
            hint={t("settings.general.skinHint")}
          >
            <div className="flex gap-1.5">
              {SKINS.map((s) => {
                const selected = prefs?.skin === s.value
                return (
                  <Tooltip key={s.value}>
                    <TooltipTrigger
                      render={
                        <button
                          type="button"
                          aria-label={s.label}
                          aria-pressed={selected}
                          data-skin={s.brand ? undefined : s.value}
                          disabled={savingSkin}
                          onClick={async () => {
                            await runWithToast(setSkin, s.value, {
                              failed: { key: "settings.toast.saveFailed" },
                            })
                          }}
                          className={cn(
                            "flex h-8 w-8 items-center justify-center rounded-md border-2 transition outline-none focus-visible:ring-2 focus-visible:ring-ring/40",
                            selected
                              ? "border-foreground"
                              : "border-transparent hover:border-border",
                          )}
                          style={{ background: s.brand ?? "var(--brand)" }}
                        />
                      }
                    >
                      {selected ? (
                        <Check
                          className="size-3.5 text-black dark:text-white"
                          style={{
                            filter: "drop-shadow(0 0 1px rgba(0, 0, 0, 0.55))",
                          }}
                        />
                      ) : null}
                    </TooltipTrigger>
                    <TooltipContent>{s.label}</TooltipContent>
                  </Tooltip>
                )
              })}
            </div>
          </SettingRow>
          <SettingRow
            label={t("settings.general.language")}
            hint={t("settings.general.languageHint")}
          >
            <Select
              value={prefs?.language}
              onValueChange={async (v) => {
                await runWithToast(setLanguage, v as Language, {
                  failed: { key: "settings.toast.saveFailed" },
                })
              }}
            >
              <SelectTrigger className="w-36" disabled={savingLang}>
                <SelectValue placeholder="—">
                  {(v: string) =>
                    LANGUAGES.find((o) => o.code === v)?.nativeName ?? "—"
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {LANGUAGES.map((o) => (
                  <SelectItem key={o.code} value={o.code}>
                    {o.nativeName}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </SettingRow>
          <SettingRow
            label={t("settings.general.lightweightExpand")}
            hint={t("settings.general.lightweightExpandHint")}
          >
            <div className="flex flex-wrap justify-end gap-2">
              {EXPAND_OPTIONS.map(([value, key]) => (
                <Button
                  key={value}
                  size="sm"
                  variant={
                    prefs?.lightweight_expand === value ? "default" : "outline"
                  }
                  disabled={savingExpand}
                  onClick={async () => {
                    await runWithToast(setLightweightExpand, value, {
                      failed: { key: "settings.toast.saveFailed" },
                    })
                  }}
                >
                  {t(key)}
                </Button>
              ))}
            </div>
          </SettingRow>
          <SettingRow
            label={t("settings.general.autoTuck")}
            hint={t("settings.general.autoTuckHint")}
          >
            <PresetSelect
              value={prefs?.lightweight_auto_tuck_secs}
              options={AUTO_TUCK_OPTIONS}
              disabled={savingAutoTuck}
              formatLabel={(s) =>
                formatDurationLabel(s, t, {
                  zeroKey: "settings.general.autoTuckOff",
                })
              }
              onChange={(v) =>
                runWithToast(setAutoTuck, v, {
                  failed: { key: "settings.toast.saveFailed" },
                })
              }
            />
          </SettingRow>
        </div>

        {/* 运行 — cadence + lifecycle behavior. ≥1240px: second column with a
            vertical divider; below: stacked with a horizontal separator. */}
        <div className="border-border/60 mt-4 flex flex-col border-t pt-4 min-[1240px]:mt-0 min-[1240px]:border-t-0 min-[1240px]:pt-0 min-[1240px]:pl-8 min-[1240px]:border-l">
          <GroupCap>{t("settings.general.groupRuntime")}</GroupCap>
          <SettingRow
            label={t("settings.general.collectInterval")}
            hint={t("settings.general.collectIntervalHint")}
          >
            <PresetSelect
              value={prefs?.collect_interval_secs}
              options={COLLECT_OPTIONS}
              disabled={savingCollect}
              formatLabel={(s) => formatDurationLabel(s, t)}
              onChange={(v) =>
                runWithToast(setCollectInterval, v, {
                  failed: { key: "settings.toast.saveFailed" },
                })
              }
            />
          </SettingRow>
          <SettingRow
            label={t("settings.general.pushInterval")}
            hint={t("settings.general.pushIntervalHint")}
          >
            {synced ? (
              <PresetSelect
                value={prefs?.push_interval_secs}
                options={PUSH_OPTIONS}
                disabled={savingPush}
                formatLabel={(s) => formatDurationLabel(s, t)}
                onChange={(v) =>
                  runWithToast(setPushInterval, v, {
                    failed: { key: "settings.toast.saveFailed" },
                  })
                }
              />
            ) : (
              /* 指向「同步」卡的提示 — 可点击滚到目标卡，而不是只指路 */
              <button
                type="button"
                onClick={() =>
                  document
                    .getElementById("sync-section")
                    ?.scrollIntoView({ behavior: "smooth" })
                }
                className="text-muted-foreground hover:text-foreground rounded-sm text-xs underline underline-offset-2 outline-none focus-visible:ring-2 focus-visible:ring-ring/40"
              >
                {t("settings.general.pushNeedsSync")}
              </button>
            )}
          </SettingRow>
          <SettingRow
            label={t("settings.general.closeBehavior")}
            hint={t("settings.general.closeBehaviorHint")}
          >
            <div className="flex flex-wrap justify-end gap-2">
              {CLOSE_OPTIONS.map(([value, key]) => (
                <Button
                  key={value}
                  size="sm"
                  variant={
                    prefs?.close_behavior === value ? "default" : "outline"
                  }
                  disabled={savingClose}
                  onClick={async () => {
                    await runWithToast(setCloseBehavior, value, {
                      failed: { key: "settings.toast.saveFailed" },
                    })
                  }}
                >
                  {t(key)}
                </Button>
              ))}
            </div>
          </SettingRow>
        </div>
      </div>
    </div>
  )
}

/** Column caption — the small eyebrow above each preference group. */
function GroupCap({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="text-muted-foreground/70 mb-1 text-[10.5px] font-semibold tracking-[0.12em]">
      {children}
    </h4>
  )
}

/**
 * 秒/分级离散预设的 Select——autoTuck / collect / push 三个逐字同构块的
 * 塌缩（架构审查Ⅵ候选⑧a）。触发器文案与选项列表共用同一个 formatLabel：
 * 档位文案只写一次，触发器显示什么、列表就显示什么，不可能再分裂。秒数
 * 是这里的通用货币（value / options / onChange 全走秒），字符串化只发生在
 * Select 的边界内；文案分档统一在 formatDurationLabel（lib/format），本
 * 组件不自己写秒/分换算。
 */
function PresetSelect({
  value,
  options,
  formatLabel,
  onChange,
  disabled,
}: {
  /** 当前选中的秒数；undefined（偏好未读到）交给 placeholder。 */
  value: number | undefined
  options: ReadonlyArray<number>
  /** 秒数 → 展示文案（触发器与选项共用同一实现）。 */
  formatLabel: (secs: number) => string
  onChange: (secs: number) => unknown
  disabled?: boolean
}) {
  return (
    <Select
      value={value === undefined ? undefined : String(value)}
      onValueChange={(v) => onChange(Number(v))}
    >
      <SelectTrigger className="w-36" disabled={disabled}>
        <SelectValue placeholder="—">
          {(v: string) => formatLabel(Number(v))}
        </SelectValue>
      </SelectTrigger>
      <SelectContent>
        {options.map((v) => (
          <SelectItem key={v} value={String(v)}>
            {formatLabel(v)}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}
