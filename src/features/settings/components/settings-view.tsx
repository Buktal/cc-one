// Settings view: device identity, run mode, repo binding
// (Standalone ↔ Synced), manual sync / rebill.
//
// Sectioned cards (通用 / 本机 / 同步 / 设备), each fronted by an eyebrow
// label. The sync card's state machine (probe / bind / unbind / sync-now +
// draft inputs) lives in useSyncRepo; this file is presentation. The sync
// card renders TWO distinct states — unbound (inputs + test/bind) vs bound
// (current repo, copyable, + test/sync/unbind) — never a mix of both.

import {
  Calculator,
  CheckCircle2,
  CloudUpload,
  Loader2,
  PlugZap,
  RefreshCw,
  Unplug,
  XCircle,
} from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useRebillMutation, useSetDisplayNameMutation } from "@/app/store/api"
import { CopyButton } from "@/components/copy-button"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { DeviceList } from "@/features/settings/components/device-list"
import { GeneralCard } from "@/features/settings/components/general-card"
import { useSyncRepo } from "@/features/settings/use-sync-repo"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import type { VerifyReport } from "@/types/generated/bindings"

export function SettingsView() {
  const { t } = useTranslation()
  const {
    info,
    synced,
    repoUrl,
    setRepoUrl,
    token,
    setToken,
    verifyResult,
    onVerify,
    bindRepo,
    unbind,
    syncNowAction,
    binding,
    clearing,
    verifying,
    syncing,
  } = useSyncRepo()
  const [setName, { isLoading: naming }] = useSetDisplayNameMutation()
  const [rebill, { isLoading: rebilling }] = useRebillMutation()
  const runWithToast = useMutateWithToast()

  const [displayName, setDisplayName] = useState("")

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-6">
      {/* 通用 — tray / language / update */}
      <Section
        eyebrow={t("settings.section.general")}
        description={t("settings.sectionDesc.general")}
      >
        <GeneralCard />
      </Section>

      {/* 本机 — identity + 补算 (rebill is a data-repair for this machine's
          records, so it docks here rather than in its own thin card) */}
      <Section
        eyebrow={t("settings.section.local")}
        description={t("settings.sectionDesc.local")}
      >
        <Row label={t("settings.local.deviceId")}>
          <span className="flex min-w-0 items-center gap-1">
            <code className="bg-muted truncate rounded px-2 py-1 font-mono text-xs">
              {info?.device_id ?? "—"}
            </code>
            {info?.device_id ? (
              <CopyButton
                value={info.device_id}
                label={t("settings.local.copyDeviceId")}
              />
            ) : null}
          </span>
        </Row>
        <Row label={t("settings.local.runMode")}>
          <Badge variant={synced ? "default" : "secondary"}>
            {synced ? t("settings.local.modeSynced") : t("shell.standalone")}
          </Badge>
        </Row>
        <Row label={t("settings.local.claudeLogDir")}>
          <span className="flex min-w-0 items-center gap-1">
            <span className="text-muted-foreground truncate font-mono text-xs">
              {info?.claude_projects_dir ?? "—"}
            </span>
            {info?.claude_projects_dir ? (
              <CopyButton
                value={info.claude_projects_dir}
                label={t("settings.local.copyLogDir")}
              />
            ) : null}
          </span>
        </Row>
        <div className="bg-border h-px" />
        <div className="flex flex-col gap-2">
          <Label className="text-muted-foreground text-xs">
            {t("settings.local.displayName")}
          </Label>
          <div className="flex items-center gap-2">
            <Input
              className="flex-1"
              placeholder={t("settings.local.displayNamePlaceholder")}
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
            />
            <Button
              size="sm"
              disabled={naming || !displayName.trim()}
              onClick={async () => {
                const ok = await runWithToast(setName, displayName.trim(), {
                  success: { key: "settings.toast.displayNameUpdated" },
                  failed: { key: "settings.toast.renameFailed" },
                })
                if (ok) setDisplayName("")
              }}
            >
              {t("common.save")}
            </Button>
          </div>
        </div>
        <div className="bg-border h-px" />
        <div className="flex items-start justify-between gap-4">
          <div className="flex min-w-0 flex-col gap-1">
            <Label className="text-foreground">
              {t("settings.maintenance.rebillLabel")}
            </Label>
            <p className="text-muted-foreground text-xs leading-relaxed">
              {t("settings.maintenance.rebillHint")}
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            disabled={rebilling}
            onClick={async () => {
              await runWithToast(rebill, undefined, {
                success: {
                  message: (count) =>
                    t("settings.toast.rebilled", { count: count ?? 0 }),
                },
                failed: { key: "settings.toast.rebillFailed" },
              })
            }}
          >
            <Calculator className="size-4" />
            {rebilling
              ? t("settings.maintenance.rebilling")
              : t("settings.maintenance.rebillButton")}
          </Button>
        </div>
      </Section>

      {/* 同步 — 仓库绑定 + 用量同步。两态分离：未绑定只展示输入与绑定；
          已绑定只展示当前配置（可复制）与操作，绝不混排。 */}
      <Section
        id="sync-section"
        eyebrow={t("settings.section.sync")}
        description={t("settings.sectionDesc.sync")}
      >
        {synced ? (
          <>
            <Row label={t("settings.sync.currentRepo")}>
              <span className="flex min-w-0 items-center gap-1">
                <span className="text-muted-foreground truncate font-mono text-xs">
                  {info?.repo_url ?? "—"}
                </span>
                {info?.repo_url ? (
                  <CopyButton
                    value={info.repo_url}
                    label={t("settings.sync.copyRepoUrl")}
                  />
                ) : null}
              </span>
            </Row>
            <Row label="Token">
              <span className="text-muted-foreground font-mono text-xs">
                {info?.masked_token ?? t("settings.sync.notConfigured")}
              </span>
            </Row>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={verifying}
                onClick={onVerify}
              >
                <PlugZap className="size-4" />
                {verifying
                  ? t("settings.sync.verifying")
                  : t("settings.sync.testConnection")}
              </Button>
              <Button size="sm" disabled={syncing} onClick={syncNowAction}>
                <RefreshCw className="size-4" />
                {syncing
                  ? t("settings.sync.syncing")
                  : t("settings.sync.syncNow")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={clearing}
                onClick={unbind}
              >
                <Unplug className="size-4" />
                {t("settings.sync.unbind")}
              </Button>
            </div>
          </>
        ) : (
          <>
            <div className="flex flex-col gap-2">
              <Label className="text-muted-foreground text-xs">
                {t("settings.sync.repoUrl")}
              </Label>
              <Input
                placeholder="https://github.com/<owner>/<repo>.git"
                value={repoUrl}
                onChange={(e) => setRepoUrl(e.target.value)}
              />
              <Label className="text-muted-foreground text-xs">
                {t("settings.sync.githubToken")}
              </Label>
              <Input
                type="password"
                placeholder="github_pat_…"
                value={token}
                onChange={(e) => setToken(e.target.value)}
              />
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={verifying || !repoUrl.trim() || !token.trim()}
                onClick={onVerify}
              >
                <PlugZap className="size-4" />
                {verifying
                  ? t("settings.sync.verifying")
                  : t("settings.sync.testConnection")}
              </Button>
              <Button
                size="sm"
                disabled={binding || !repoUrl.trim() || !token.trim()}
                onClick={bindRepo}
              >
                <CloudUpload className="size-4" />
                {t("settings.sync.bindAndEnable")}
              </Button>
            </div>
          </>
        )}
        {(verifying || verifyResult) && (
          <VerifyBanner verifying={verifying} result={verifyResult} />
        )}
      </Section>

      {/* 设备 — 同步过的设备（同步的产物，故排在同步卡之后） */}
      <Section
        eyebrow={t("settings.section.devices")}
        description={t("settings.sectionDesc.devices")}
      >
        <DeviceList />
      </Section>
    </div>
  )
}

function Section({
  id,
  eyebrow,
  description,
  children,
}: {
  id?: string
  eyebrow: string
  description?: string
  children: React.ReactNode
}) {
  return (
    <section id={id} className="flex scroll-mt-4 flex-col gap-2.5">
      <div className="flex flex-col gap-1 px-0.5">
        <h2 className="text-muted-foreground text-[11px] font-semibold tracking-[0.14em]">
          {eyebrow}
        </h2>
        {description ? (
          <p className="text-muted-foreground/70 text-xs leading-relaxed">
            {description}
          </p>
        ) : null}
      </div>
      <Card interactive>
        <CardContent className="flex flex-col gap-3">{children}</CardContent>
      </Card>
    </section>
  )
}

function Row({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex items-center justify-between gap-4">
      <span className="text-muted-foreground shrink-0 text-sm">{label}</span>
      {children}
    </div>
  )
}

/** 测试连接结果 banner（诊断型操作，结果需持久可见，故用 inline 而非 toast）。
 *  `result.message` 来自 Rust 后端，保持英文不本地化。 */
function VerifyBanner({
  verifying,
  result,
}: {
  verifying: boolean
  result: VerifyReport | null
}) {
  const { t } = useTranslation()
  if (verifying) {
    return (
      <div className="bg-muted/50 text-muted-foreground flex items-center gap-2 rounded-md border border-dashed p-2 text-xs">
        <Loader2 className="size-3.5 animate-spin" />
        {t("settings.sync.verifyingBanner")}
      </div>
    )
  }
  if (!result) return null
  if (result.ok) {
    return (
      <div className="border-emerald-500/40 bg-emerald-500/5 text-emerald-600 dark:text-emerald-400 flex flex-col gap-0.5 rounded-md border p-2 text-xs leading-relaxed">
        <span className="flex items-start gap-2">
          <CheckCircle2 className="mt-0.5 size-3.5 shrink-0" />
          {result.message}
        </span>
        <span className="text-muted-foreground pl-5">
          {t("settings.sync.verifyReadPermNote")}
        </span>
      </div>
    )
  }
  return (
    <div className="border-destructive/40 bg-destructive/5 text-destructive flex items-start gap-2 rounded-md border p-2 text-xs leading-relaxed">
      <XCircle className="mt-0.5 size-3.5 shrink-0" />
      <span>{result.message}</span>
    </div>
  )
}
