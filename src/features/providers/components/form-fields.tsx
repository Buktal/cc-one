// 表单共享小组件：Field（标签 + 控件竖排）与 BasicSection（基本信息分区：
// 分区标题 + 名称输入）。provider-form-sheet 的 claude 分区独立成组件后，名称
// 字段在父组件（非 claude 分支）与 claude 分区各渲染一份——收进本文件单一
// 实现，避免两份分叉（opencode-form-fields 曾各自内联 Field）。

import { useTranslation } from "react-i18next"
import { SectionHeader } from "@/components/section-header"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { cn } from "@/lib/utils"

/** 表单字段：Label（muted xs）+ 控件竖排。className 用于布局（flex 子项的
 *  伸缩 / 宽度约束）。 */
export function Field({
  label,
  className,
  children,
}: {
  label: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <Label className="text-muted-foreground text-xs">{label}</Label>
      {children}
    </div>
  )
}

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
