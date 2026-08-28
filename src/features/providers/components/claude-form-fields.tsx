// Claude 供应商表单分区（AppProfile.formPartition 的 claude 行）：模板变量
// 输入区 + 基本信息（名称 / 端点 / 认证键）+ 模型映射（自动应用开关、拉模
// 型列表、一键设置、五角色表）。真相源是 configText（settingsConfig JSON
// 文本）：字段读直 derive（configEndpoint / configApiKey / configAuthField /
// configRoleFields，无镜像 state），写回经父组件传入的守卫 onChange（仅当
// JSON 合法时写——半截 JSON 不被吞，见 lib/json 的 guardedRewrite）。auth
// 字段的显示条件与模板变量区同属本分区（云厂商预设认证走模板变量，不显示
// key 输入框）。拉模型管线由父组件注入（与 gemini / opencode 共用同一份
// runFetchModels，避免分叉漂移）；表单态经 form 组进分区（契约见
// form-partition.ts）。

import { RefreshCw, Wand2 } from "lucide-react"
import { Fragment, useMemo } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Field } from "@/components/form-field"
import { SectionHeader } from "@/components/section-header"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import {
  type AuthField,
  authFieldKey,
  configApiKey,
  configAuthField,
  configEndpoint,
  configRoleFields,
  MODEL_ROLES,
  type ModelRoleId,
  stripOneM,
  switchAuthField,
  withAllRolesFromFirstInText,
  withAllRolesInText,
  withBasicFieldsInText,
  withRoleModelInText,
  withRoleNameInText,
  withRoleOneMInText,
} from "@/features/providers/codecs/claude"
import { BasicSection } from "@/features/providers/components/form-fields"
import { ModelPickSelect } from "@/features/providers/components/model-pick-select"
import type { FormPartitionProps } from "@/features/providers/form-partition"
import { extractTemplateVars } from "@/features/providers/template-vars"
import { parseJsonObject } from "@/lib/json"
import { cn } from "@/lib/utils"

export function ClaudeFormFields({
  configText,
  onChange,
  form,
  models,
}: FormPartitionProps) {
  const { t } = useTranslation()
  const {
    name,
    onNameChange,
    templateValues,
    onTemplateVarChange,
    autoSync,
    onAutoSyncChange,
    category,
  } = form
  const { fetching, fetchedModels, onFetchModels, onEndpointEdited } = models

  // The five model roles are derived straight from the snapshot text (no
  // mirrored state to keep in sync): reads are pure, and every write goes
  // through the derive helpers below, so the JSON editor and the role rows can
  // never disagree. Garbage text derives to empty rows and blocks writes until
  // it parses again.
  const roleRows = useMemo(
    () =>
      MODEL_ROLES.map((role) => ({
        role,
        fields: configRoleFields(configText, role.id),
      })),
    [configText],
  )

  const canApplyAll = useMemo(() => {
    if (!parseJsonObject(configText).ok) return false
    return withAllRolesFromFirstInText(configText) !== null
  }, [configText])

  // 模板变量输入区：快照文本里出现的 `${VAR}` 占位符 + meta 里记录的已填值
  // （重编时占位符已物化，输入框仍要显示）。无模板变量的普通供应商整段隐藏。
  const templateVarNames = useMemo(
    () =>
      Array.from(
        new Set([
          ...extractTemplateVars(configText),
          ...Object.keys(templateValues),
        ]),
      ),
    [configText, templateValues],
  )
  // key 输入区显示条件：快照带模板变量（云厂商的认证走模板变量）或分类为官方
  // /云厂商时隐藏，普通供应商照常显示。
  const showKeyFields =
    templateVarNames.length === 0 &&
    category !== "official" &&
    category !== "cloud_provider"

  // 字段写回：改一个字段时，其余字段从 prev（待写文本）派生读回——与旧镜像
  // 态同值（镜像与文本解析态等价），但规则在 codec（可测），不散在组件里。
  function onEndpointChange(value: string) {
    // 端点变了，旧端点拉到的模型列表不再可靠，清空下拉。
    onEndpointEdited()
    onChange((prev) =>
      withBasicFieldsInText(prev, {
        endpoint: value,
        apiKey: configApiKey(prev),
        authField: configAuthField(prev),
      }),
    )
  }

  function onApiKeyChange(value: string) {
    onChange((prev) =>
      withBasicFieldsInText(prev, {
        endpoint: configEndpoint(prev),
        apiKey: value,
        authField: configAuthField(prev),
      }),
    )
  }

  function onAuthFieldChange(to: AuthField) {
    if (to === configAuthField(configText)) return
    // 值搬到新键、旧键删除——切换不丢 key 也不留双拼写。
    onChange((prev) => switchAuthField(prev, configAuthField(prev), to))
  }

  function onRoleModelChange(role: ModelRoleId, value: string) {
    // 自动应用开关开着时：编辑任一角色模型 → 同步全部角色（withAllRolesInText
    // 处理 Haiku 去标记、显示名跟随）。关着则只改当前角色。
    onChange((prev) =>
      autoSync
        ? withAllRolesInText(prev, value)
        : withRoleModelInText(prev, role, value),
    )
  }

  function onRoleNameChange(role: ModelRoleId, value: string) {
    onChange((prev) => withRoleNameInText(prev, role, value))
  }

  function onRoleOneMChange(role: ModelRoleId, oneM: boolean) {
    onChange((prev) => withRoleOneMInText(prev, role, oneM))
  }

  function onApplyAll() {
    const next = withAllRolesFromFirstInText(configText)
    if (next === null) return
    if (onChange(() => next)) {
      toast.success(t("providers.toast.applyAllSuccess"))
    }
  }

  /** 下拉选中一个模型 → 回填五个角色（与「一键设置」同一写入引擎）。 */
  function onPickModel(model: string) {
    if (onChange((prev) => withAllRolesInText(prev, model))) {
      toast.success(t("providers.toast.applyAllSuccess"))
    }
  }

  return (
    <>
      {templateVarNames.length > 0 ? (
        /* 与其他分区同一语言：SectionHeader 小标题 + 说明文字，不套盒子
           （分区靠间距与字号，不靠底块——与模型映射一致）。变量多时 2 列
           网格紧凑排列。 */
        <>
          <SectionHeader className="mt-0">
            {t("providers.form.templateVars")}
          </SectionHeader>
          <p className="text-muted-foreground -mt-1.5 text-xs">
            {t("providers.form.templateVarsHint")}
          </p>
          <div className="grid grid-cols-2 gap-x-3 gap-y-2">
            {templateVarNames.map((name) => (
              <Field key={name} label={name}>
                <Input
                  value={templateValues[name] ?? ""}
                  onChange={(e) => onTemplateVarChange(name, e.target.value)}
                  spellCheck={false}
                />
              </Field>
            ))}
          </div>
        </>
      ) : null}
      <BasicSection name={name} onNameChange={onNameChange} />
      <Field label={t("providers.form.endpoint")}>
        <Input
          value={configEndpoint(configText)}
          onChange={(e) => onEndpointChange(e.target.value)}
          placeholder="https://api.example.com"
          spellCheck={false}
        />
      </Field>
      {showKeyFields ? (
        <div className="flex items-end gap-2">
          <Field label={t("providers.form.authField")} className="shrink-0">
            <Select
              value={configAuthField(configText)}
              onValueChange={(v) => {
                if (v) onAuthFieldChange(v as AuthField)
              }}
            >
              <SelectTrigger
                className="w-56 font-mono text-xs"
                aria-label={t("providers.form.authField")}
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="auth_token">
                  {authFieldKey("auth_token")}
                </SelectItem>
                <SelectItem value="api_key">
                  {authFieldKey("api_key")}
                </SelectItem>
              </SelectContent>
            </Select>
          </Field>
          <Field label={t("providers.form.apiKey")} className="min-w-0 flex-1">
            <Input
              type="password"
              value={configApiKey(configText)}
              onChange={(e) => onApiKeyChange(e.target.value)}
              placeholder={t("providers.form.apiKeyPlaceholder")}
              spellCheck={false}
            />
          </Field>
        </div>
      ) : null}
      {/* 分区标题与操作按钮同处一行（SectionHeader 的 action 槽），
          与「基本信息 / 高级配置」同一分区语言。 */}
      <SectionHeader
        action={
          <div className="flex items-center gap-2">
            {/* 自动应用开关：开着时编辑任一角色模型自动同步全部角色
              （新建/复制默认开，编辑默认关）。与「一键设置」并存——
              开关管持续行为，按钮管一次性统一。 */}
            <label
              htmlFor="model-auto-sync"
              className="flex cursor-pointer items-center gap-1.5 text-xs text-muted-foreground"
            >
              <Switch
                id="model-auto-sync"
                checked={autoSync}
                onCheckedChange={onAutoSyncChange}
              />
              {t("providers.form.autoSync")}
            </label>
            <Button
              variant="outline"
              size="sm"
              onClick={onFetchModels}
              disabled={fetching}
            >
              <RefreshCw
                className={cn("size-3.5", fetching && "animate-spin")}
              />
              {fetching
                ? t("providers.form.fetchModels.fetching")
                : t("providers.form.fetchModels.fetch")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={onApplyAll}
              disabled={!canApplyAll}
            >
              <Wand2 className="size-3.5" />
              {t("providers.form.applyAll")}
            </Button>
          </div>
        }
      >
        {t("providers.form.modelMapping")}
      </SectionHeader>
      <p className="text-muted-foreground -mt-1.5 text-xs">
        {t("providers.form.modelMappingHint")}
      </p>
      <ModelPickSelect
        models={fetchedModels}
        placeholder={t("providers.form.fetchModels.placeholder")}
        onPick={onPickModel}
        className="max-w-sm"
      />
      {/* 角色表：列头 + 一行一角色（角色名+1M | 显示名 | 请求模型）。表格形
          而非灰盒子——列对齐让 5 行角色可纵向扫读，行距压缩一半（原每角色占
          两行）；不加行分隔线（减线）。列头保持 11px semibold（表列标签尺度，
          独立于分区标题）。 */}
      <div className="grid grid-cols-[minmax(8rem,9.5rem)_1fr_1fr] items-center gap-x-3 gap-y-2">
        <div className="text-muted-foreground text-[11px] font-semibold">
          {t("providers.form.role")}
        </div>
        <div className="text-muted-foreground text-[11px] font-semibold">
          {t("providers.form.displayName")}
        </div>
        <div className="text-muted-foreground text-[11px] font-semibold">
          {t("providers.form.requestModel")}
        </div>
        {roleRows.map(({ role, fields }) => (
          <Fragment key={role.id}>
            <div className="flex h-8 items-center justify-between gap-1">
              <span className="text-muted-foreground text-xs">
                {t(`providers.form.role.${role.id}`)}
              </span>
              {role.supportsOneM ? (
                <label
                  htmlFor={`model-role-one-m-${role.id}`}
                  className="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground"
                >
                  <Checkbox
                    id={`model-role-one-m-${role.id}`}
                    checked={fields.oneM}
                    onCheckedChange={(checked) =>
                      onRoleOneMChange(role.id, checked)
                    }
                  />
                  {t("providers.form.oneM")}
                </label>
              ) : null}
            </div>
            <Input
              value={fields.name}
              onChange={(e) => onRoleNameChange(role.id, e.target.value)}
              placeholder={stripOneM(fields.model)}
              spellCheck={false}
            />
            <Input
              value={fields.model}
              onChange={(e) => onRoleModelChange(role.id, e.target.value)}
              spellCheck={false}
            />
          </Fragment>
        ))}
      </div>
    </>
  )
}
