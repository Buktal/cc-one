// 通用配置片段卡片（供应商视图页脚）：全局一条、跨供应商共享的
// settings.json 片段 + 启用开关。保存走 set_common_config_snippet_cmd
// （后端校验 JSON 合法）；切换写盘时由 switch_provider_cmd 把启用的片段
// 合并进受控字段（供应商显式配置优先，非受控键忽略）。卡片底部的提示用
// snippetMissingKeys 对「当前激活供应商」做子集判定——告诉用户这次切换
// 片段实际会补上什么（或已全部覆盖）。

import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useGetActiveProviderQuery,
  useGetCommonConfigSnippetQuery,
  useSetCommonConfigSnippetMutation,
} from "@/app/store/api"
import { JsonEditor } from "@/components/json-editor"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { snippetMissingKeys } from "@/features/providers/derive"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { parseJsonObject } from "@/lib/json"

import type { App, CommonConfigSnippet } from "@/types/generated/bindings"

export function CommonConfigSnippetCard({ app }: { app: App }) {
  const { t } = useTranslation()
  // 通用配置片段按应用独立：claude / codex / gemini 各一份。
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

  async function onSave() {
    const parsed = parseJsonObject(content)
    if (!parsed.ok) {
      toast.error(t("providers.snippet.invalidJson"), {
        description: parsed.error,
      })
      return
    }
    await runWithToast(
      save,
      {
        app,
        snippet: { enabled, content } satisfies CommonConfigSnippet,
      },
      {
        success: { key: "providers.snippet.saved" },
        failed: { key: "providers.snippet.saveFailed" },
      },
    )
  }

  // 子集判定提示：对当前激活供应商，片段会补上什么。
  const missingKeys = activeProvider
    ? snippetMissingKeys(activeProvider.settingsConfig, content)
    : []

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
            onCheckedChange={setEnabled}
          />
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <p className="text-muted-foreground text-xs">
          {t("providers.snippet.enabledHint")}
        </p>
        <JsonEditor
          value={content}
          onChange={setContent}
          placeholder={t("providers.snippet.jsonPlaceholder")}
          className="h-40"
        />
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted-foreground truncate text-xs">
            {activeProvider
              ? missingKeys.length > 0
                ? t("providers.snippet.deltaHint", {
                    keys: missingKeys.join(", "),
                  })
                : t("providers.snippet.coveredHint")
              : t("providers.snippet.noActiveHint")}
          </span>
          <Button size="sm" disabled={saving || isLoading} onClick={onSave}>
            {saving ? t("common.saving") : t("providers.snippet.save")}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}
