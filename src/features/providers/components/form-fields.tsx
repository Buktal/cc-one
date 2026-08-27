// 表单共享小组件：BasicSection（基本信息分区：分区标题 + 名称输入）。
// provider-form-sheet 的 claude 分区独立成组件后，名称字段在父组件（非
// claude 分支）与 claude 分区各渲染一份——收进本文件单一实现，避免两份
// 分叉。Field 原子已下放 @/components/form-field（架构审查Ⅲ候选⑫）。

import { useTranslation } from "react-i18next"

import { Field } from "@/components/form-field"
import { SectionHeader } from "@/components/section-header"
import { Input } from "@/components/ui/input"

/** 基本信息分区：分区标题 + 名称输入。所有 app 共用（claude 分区组件渲染在
 *  模板变量区之后，父表单的非 claude 分支渲染在字段区之前）——单一实现。 */
export function BasicSection({
  name,
  onNameChange,
}: {
  name: string
  onNameChange: (value: string) => void
}) {
  const { t } = useTranslation()
  return (
    <>
      <SectionHeader>{t("providers.form.section.basic")}</SectionHeader>
      <Field label={t("providers.form.name")}>
        <Input
          value={name}
          onChange={(e) => onNameChange(e.target.value)}
          placeholder={t("providers.form.namePlaceholder")}
        />
      </Field>
    </>
  )
}
