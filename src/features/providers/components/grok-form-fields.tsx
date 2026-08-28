// Grok 供应商表单分区（AppProfile.formPartition 的 grok 行）：基本信息
// （名称）+ config.toml 编辑器。字段直写 configText（读直 derive 的 codec，
// 写回经父组件传入的守卫 onChange——仅当 settingsConfig JSON 合法时写，半
// 截 JSON 不被吞）。无拉模型入口（app-profiles 的 modelFetch 行为 null），
// models 管线不解构。

import { useTranslation } from "react-i18next"
import { Field } from "@/components/form-field"
import { Textarea } from "@/components/ui/textarea"
import {
  grokConfigToml,
  withGrokConfigToml,
} from "@/features/providers/codecs/grok"
import { BasicSection } from "@/features/providers/components/form-fields"
import type { FormPartitionProps } from "@/features/providers/form-partition"

export function GrokFormFields({
  configText,
  onChange,
  form,
}: FormPartitionProps) {
  const { t } = useTranslation()
  const { name, onNameChange } = form
  return (
    <>
      <BasicSection name={name} onNameChange={onNameChange} />
      <Field label={t("providers.form.grokConfig")}>
        <Textarea
          value={grokConfigToml(configText)}
          onChange={(e) =>
            onChange((prev) => withGrokConfigToml(prev, e.target.value))
          }
          rows={12}
          spellCheck={false}
          className="font-mono text-xs"
        />
        <p className="text-muted-foreground text-xs">
          {t("providers.form.grokConfigHint")}
        </p>
      </Field>
    </>
  )
}
