// Gemini 供应商表单分区（AppProfile.formPartition 的 gemini 行）：基本信息
// （名称）+ API Key + 模型（拉模型入口 + 下拉候选）+ 端点。字段直写
// configText（读直 derive 的 codec，写回经父组件传入的守卫 onChange——仅
// 当 settingsConfig JSON 合法时写，半截 JSON 不被吞）。拉模型管线由父组件
// 注入（共享 runFetchModels 的错误分桶，避免分叉）。

import { RefreshCw } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Field } from "@/components/form-field"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  geminiApiKey,
  geminiBaseUrl,
  geminiModel,
  withGeminiEnv,
} from "@/features/providers/codecs/gemini"
import { BasicSection } from "@/features/providers/components/form-fields"
import { ModelPickSelect } from "@/features/providers/components/model-pick-select"
import type { FormPartitionProps } from "@/features/providers/form-partition"
import { cn } from "@/lib/utils"

export function GeminiFormFields({
  configText,
  onChange,
  form,
  models,
}: FormPartitionProps) {
  const { t } = useTranslation()
  const { name, onNameChange } = form
  const { fetching, fetchedModels, onFetchModels, onEndpointEdited } = models

  /** Gemini 下拉选中一个模型 → 写入 GEMINI_MODEL（Gemini 只有一个模型字段）。 */
  function onPickModel(model: string) {
    if (onChange((prev) => withGeminiEnv(prev, { GEMINI_MODEL: model }))) {
      toast.success(t("providers.toast.fetchModels.modelSet"))
    }
  }

  return (
    <>
      <BasicSection name={name} onNameChange={onNameChange} />
      <Field label={t("providers.form.apiKey")}>
        <Input
          type="password"
          value={geminiApiKey(configText)}
          onChange={(e) =>
            onChange((prev) =>
              withGeminiEnv(prev, { GEMINI_API_KEY: e.target.value }),
            )
          }
          placeholder={t("providers.form.apiKeyPlaceholder")}
          spellCheck={false}
        />
      </Field>
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <Label className="text-muted-foreground text-xs">
            {t("providers.form.geminiModel")}
          </Label>
          <Button
            variant="outline"
            size="sm"
            onClick={onFetchModels}
            disabled={fetching}
          >
            <RefreshCw className={cn("size-3.5", fetching && "animate-spin")} />
            {fetching
              ? t("providers.form.fetchModels.fetching")
              : t("providers.form.fetchModels.fetch")}
          </Button>
        </div>
        <Input
          value={geminiModel(configText)}
          onChange={(e) =>
            onChange((prev) =>
              withGeminiEnv(prev, { GEMINI_MODEL: e.target.value }),
            )
          }
          spellCheck={false}
        />
        <ModelPickSelect
          models={fetchedModels}
          placeholder={t("providers.form.fetchModels.geminiPlaceholder")}
          onPick={onPickModel}
        />
      </div>
      <Field label={t("providers.form.geminiBaseUrl")}>
        <Input
          value={geminiBaseUrl(configText)}
          onChange={(e) => {
            // 端点变了，旧端点拉到的模型列表不再可靠，清空下拉（与 claude
            // 分区的 onEndpointChange 同一处理）。
            onEndpointEdited()
            onChange((prev) =>
              withGeminiEnv(prev, { GOOGLE_GEMINI_BASE_URL: e.target.value }),
            )
          }}
          placeholder="https://generativelanguage.googleapis.com"
          spellCheck={false}
        />
      </Field>
    </>
  )
}
