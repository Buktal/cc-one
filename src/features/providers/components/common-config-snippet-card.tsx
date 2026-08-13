// 通用配置片段卡片（供应商视图页脚）：按应用独立一份、跨该应用供应商共享
// 的默认配置 + 启用开关。保存走 set_common_config_snippet_cmd（后端按应用校
// 验）；切换写盘时由 switch_provider_cmd 按应用分派合并层（ADR-0010）——
// claude/gemini 在 settings_config 层并入、codex/grok 在写盘层补缺失进 live
// 文件。供应商显式配置优先，片段只补缺失键。
//
// 编辑器按应用切换语言：claude/gemini 用 JSON（客户端 JSON 校验 + 格式化），
// codex/grok 用 TOML（仅高亮，合法性 + 身份键由后端校验）。卡片底部的提示按
// 应用给不同信息——claude 对当前激活供应商做片段子集判定，codex/grok 列禁
// 身份键。

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useGetActiveProviderQuery,
  useGetCommonConfigSnippetQuery,
  useSetCommonConfigSnippetMutation,
} from "@/app/store/api"
import { CodeEditor } from "@/components/code-editor"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import {
  geminiSnippetIssue,
  snippetMissingKeys,
} from "@/features/providers/derive"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { formatJson, parseJsonObject } from "@/lib/json"

import type { App, CommonConfigSnippet } from "@/types/generated/bindings"

/** codex/grok 片段写 TOML（写盘层补缺失进 config.toml）；claude/gemini 写 JSON。 */
function isTomlApp(app: App): boolean {
  return app === "codex" || app === "grok"
}

export function CommonConfigSnippetCard({ app }: { app: App }) {
  const { t } = useTranslation()
  const isToml = isTomlApp(app)
  // 通用配置片段按应用独立：claude / codex / gemini / grok 各一份。
  const { data: snippet, isLoading } = useGetCommonConfigSnippetQuery(app)
  const { data: activeProvider } = useGetActiveProviderQuery(app)
  const [save, { isLoading: saving }] = useSetCommonConfigSnippetMutation()
  const runWithToast = useMutateWithToast()

  // Local draft: synced from the persisted record when it loads / after save;
  // the editor edits the draft, only Save persists.
  const [enabled, setEnabled] = useState(false)
  const [content, setContent] = useState("")

  useEffect(() => {
    if (!snippet) return
    setEnabled(snippet.enabled)
    setContent(snippet.content)
  }, [snippet])

  // 保存（内容 / 开关共用）：JSON 应用保存前格式化为多行展开（后端存展开版，
  // 回读时天然展开）；TOML 应用发原文（无客户端格式化，合法性交后端）。JSON
  // 校验失败返回 false（调用方决定回滚）。
  async function persist(next: {
    enabled: boolean
    content: string
  }): Promise<boolean> {
    if (!isToml) {
      const parsed = parseJsonObject(next.content)
      if (!parsed.ok) {
        toast.error(t("providers.snippet.invalidJson"), {
          description: parsed.error,
        })
        return false
      }
    }
    const payload = isToml ? next.content : formatJson(next.content)
    return await runWithToast(
      save,
      {
        app,
        snippet: {
          enabled: next.enabled,
          content: payload,
        } satisfies CommonConfigSnippet,
      },
      {
        success: { key: "providers.snippet.saved" },
        failed: { key: "providers.snippet.saveFailed" },
      },
    )
  }

  async function onSave() {
    const ok = await persist({ enabled, content })
    // JSON 应用：保存后把草稿同步成展开版（与后端回读一致）；TOML 不格式化。
    if (ok && !isToml) setContent(formatJson(content))
  }

  // 开关翻转即写盘：启用状态是「生效」开关（切换供应商时后端按它决定是否
  // 合并片段），不依赖下方的「保存」按钮——否则开了开关不点保存，切供应商
  // 时片段不生效，开关形同虚设。内容非法则拒绝翻转（保存失败 + 回滚）。
  // 内容编辑仍走保存按钮。
  async function onToggleEnabled(checked: boolean): Promise<void> {
    setEnabled(checked)
    const ok = await persist({ enabled: checked, content })
    if (!ok) setEnabled(!checked)
  }

  // 子集判定提示：claude 专用（snippetMissingKeys 镜像 claude 受控字段）。
  const missingKeys =
    app === "claude" && activeProvider
      ? snippetMissingKeys(activeProvider.settingsConfig, content)
      : []

  // 底部提示按应用：claude 子集判定；codex/grok 列禁身份键；gemini 动态检测
  // 凭据/端点键（TS 镜像后端 is_sensitive_config_key，ADR-0010）——有则警告该键
  // 保存将被拒，无则列允许的键。
  const bottomHint = (() => {
    if (app === "codex") return t("providers.snippet.codexIdentityHint")
    if (app === "grok") return t("providers.snippet.grokIdentityHint")
    if (app === "gemini") {
      const issue = geminiSnippetIssue(content)
      return issue
        ? t("providers.snippet.geminiCredentialWarn", { key: issue })
        : t("providers.snippet.geminiCredentialHint")
    }
    if (app === "claude") {
      if (!activeProvider) return t("providers.snippet.noActiveHint")
      return missingKeys.length > 0
        ? t("providers.snippet.deltaHint", { keys: missingKeys.join(", ") })
        : t("providers.snippet.coveredHint")
    }
    return ""
  })()

  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between gap-2">
        <CardTitle className="text-sm">
          {t("providers.snippet.title")}
        </CardTitle>
        <div className="flex items-center gap-2">
          <Label
            htmlFor="common-snippet-enabled"
            className="text-muted-foreground text-xs"
          >
            {t("providers.snippet.enabledLabel")}
          </Label>
          <Switch
            id="common-snippet-enabled"
            checked={enabled}
            disabled={isLoading}
            onCheckedChange={(c) => void onToggleEnabled(c)}
          />
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <p className="text-muted-foreground text-xs">
          {t("providers.snippet.enabledHint")}
        </p>
        <CodeEditor
          language={isToml ? "toml" : "json"}
          value={content}
          onChange={setContent}
          placeholder={t(
            isToml
              ? "providers.snippet.tomlPlaceholder"
              : "providers.snippet.jsonPlaceholder",
          )}
          className="h-40"
        />
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground truncate text-xs">
            {bottomHint}
          </span>
          <Button size="sm" disabled={saving || isLoading} onClick={onSave}>
            {saving ? t("common.saving") : t("providers.snippet.save")}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
