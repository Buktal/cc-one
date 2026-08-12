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
import { formatJson, parseJsonObject } from "@/lib/json"

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

  // 保存（内容 / 开关共用）：**保存前格式化为多行展开**——后端存展开版，
  // 回读时天然展开。否则手输 / 编辑的单行紧凑 JSON 保存后重开时，编辑器
  // 文档恰好等于后端返回的同一份单行文本，JsonEditor 的「外部值进入才展开」
  // 会被 `cur === value` 提前跳过，紧凑单行就永远留着（加载路径只覆盖从未
  // 编辑过的内容）。校验失败返回 false（调用方决定回滚）。
  async function persist(next: {
    enabled: boolean
    content: string
  }): Promise<boolean> {
    const parsed = parseJsonObject(next.content)
    if (!parsed.ok) {
      toast.error(t("providers.snippet.invalidJson"), {
        description: parsed.error,
      })
      return false
    }
    const formatted = formatJson(next.content)
    return await runWithToast(
      save,
      {
        app,
        snippet: {
          enabled: next.enabled,
          content: formatted,
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
    if (ok) setContent(formatJson(content))
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
            onCheckedChange={(c) => void onToggleEnabled(c)}
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
