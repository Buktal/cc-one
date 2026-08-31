// Settings view: device identity, run mode, repo binding
// (Standalone ↔ Synced), manual sync / rebill.
//
// Sectioned cards (通用 / 本机 / 同步 / 设备 / 关于), each fronted by an eyebrow
// label. #109: the page widens to max-w-6xl (决议 #99 variant-a — 加宽单列，
// 「通用」卡内部两栏见 general-card.tsx) and gains the 关于 section that
// inherited the changelog + version display from the old shell footer. The
// sync card's state machine (probe / bind / unbind / sync-now + draft inputs)
// lives in useSyncRepo; this file is presentation. The sync card renders TWO
// distinct states — unbound (inputs + test/bind) vs bound (current repo,
// copyable, + test/sync/unbind) — never a mix of both.

import {
  Calculator,
  CloudUpload,
  PlugZap,
  RefreshCw,
  Unplug,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { useRebillMutation, useSetDisplayNameMutation } from "@/app/store/api"
import { CopyButton } from "@/components/copy-button"
import { InlineBanner } from "@/components/inline-banner"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent } from "@/components/ui/card"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { AboutCard } from "@/features/settings/components/about-card"
import { DeviceList } from "@/features/settings/components/device-list"
import { GeneralCard } from "@/features/settings/components/general-card"
import { useSyncRepo } from "@/features/settings/use-sync-repo"
import { useInlineEdit } from "@/hooks/use-inline-edit"
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
  const [setName] = useSetDisplayNameMutation()
  const [rebill, { isLoading: rebilling }] = useRebillMutation()
  const runWithToast = useMutateWithToast()

  // 显示名设置是常开编辑（无键位、无取消）：useInlineEdit 以 K = void 使用，
  // target / begin / cancel 不参与，机器只供「草稿 + busy + 成功清空」——
  // 提交成功 settle 即清空草稿（与旧 setDisplayName("") 同一收尾）。
  const displayName = useInlineEdit<void>({
    commit: (_target, draft) =>
      runWithToast(setName, draft.trim(), {
        success: { key: "settings.toast.displayNameUpdated" },
        failed: { key: "settings.toast.renameFailed" },
      }),
  })

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-6">
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
              value={displayName.draft}
              onChange={(e) => displayName.setDraft(e.target.value)}
            />
            <Button
              size="sm"
              disabled={displayName.busy || !displayName.draft.trim()}
              onClick={() => void displayName.commit()}
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
            <Row label={t("settings.sync.token")}>
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
                // 占位符是 HTTPS 仓库地址的格式示例——URL 本身跨语言可读，
                // 属于不本地化的专名例外（对照 VerifyBanner 的后端 message）。
                placeholder="https://github.com/<owner>/<repo>.git"
                value={repoUrl}
                onChange={(e) => setRepoUrl(e.target.value)}
              />
              <Label className="text-muted-foreground text-xs">
                {t("settings.sync.githubToken")}
              </Label>
              <Input
                type="password"
                // 占位符展示 GitHub fine-grained PAT 的字面前缀，是令牌格式
                // 示例而非英文文案，保持原文不本地化（同上属专名例外）。
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

      {/* 关于 — 版本与更新 + 更新日志（#109 承接自 shell footer） */}
      <Section
        eyebrow={t("settings.section.about")}
        description={t("settings.sectionDesc.about")}
      >
        <AboutCard />
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
 *  `result.message` 来自 Rust 后端，保持英文不本地化。本组件只做状态 → tone
 *  的分派（验证中 = busy / 成功 = success / 失败 = error），视觉配方归
 *  InlineBanner；ok 的附注行（只读权限说明）是此处独有的第二行内容。 */
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
      <InlineBanner tone="busy">
        {t("settings.sync.verifyingBanner")}
      </InlineBanner>
    )
  }
  if (!result) return null
  if (result.ok) {
    return (
      <InlineBanner tone="success">
        <span className="block">{result.message}</span>
        <span className="text-muted-foreground block pl-5">
          {t("settings.sync.verifyReadPermNote")}
        </span>
      </InlineBanner>
    )
  }
  return <InlineBanner tone="error">{result.message}</InlineBanner>
}
