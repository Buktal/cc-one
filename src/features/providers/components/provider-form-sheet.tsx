// Provider editor as a Sheet (side panel) — new and edit both flow through
// here. The basic form owns the name, the endpoint and the API key (written
// under a switchable auth field — AUTH_TOKEN by default, API_KEY for legacy
// spellings), plus the template-variable inputs on top when the snapshot
// carries `${VAR}` placeholders (the Bedrock presets); the model mapping
// section owns the five role models (Sonnet / Opus / Haiku / Fable / Subagent),
// their display names and the 1M toggles, plus the one-click apply button and
// the model-list fetch button (fetches the current provider's models into a
// dropdown; picking one refills all five roles, failures are bucketed into a
// toast). On
// save the form maps everything onto the provider's settingsConfig snapshot via
// the derive.ts helpers (withBasicFields preserves every field the form
// doesn't own), then calls the upsert mutation and closes.
//
// The settings.json editor at the bottom makes the JSON snapshot the single
// source of truth: editing the JSON re-derives the endpoint / API key / auth
// field and the role rows (they are a quick view of `env`), and editing a field
// merges it back into the JSON. The merge is guarded by `parseJsonObject` —
// while the JSON text does not parse to an object, field edits only update the
// field state and the in-progress JSON edit is never clobbered; once it
// parses again the fields re-derive. Save still goes through withBasicFields,
// so the field values win for the keys they own and everything else survives
// verbatim.

import { RefreshCw, Wand2 } from "lucide-react"
import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useFetchModelsMutation,
  useSaveProviderMutation,
} from "@/app/store/api"
import { JsonEditor } from "@/components/json-editor"
import { SectionHeader } from "@/components/section-header"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Textarea } from "@/components/ui/textarea"
import { OpenCodeFormFields } from "@/features/providers/components/opencode-form-fields"
import { PresetPicker } from "@/features/providers/components/preset-picker"
import type { ModelRoleId } from "@/features/providers/derive"
import {
  type AuthField,
  authFieldKey,
  codexApiKey,
  codexConfigToml,
  configApiKey,
  configAuthField,
  configEndpoint,
  configRoleFields,
  emptyProvider,
  extractTemplateVars,
  geminiApiKey,
  geminiBaseUrl,
  geminiModel,
  grokConfigToml,
  MODEL_ROLES,
  metaTemplateValues,
  openCodeApiKey,
  openCodeBaseUrl,
  providerApiKey,
  providerEndpoint,
  providerFromPreset,
  replaceTemplateVarsInText,
  restoreTemplatePlaceholders,
  stripOneM,
  switchAuthField,
  withAllRolesFromFirstInText,
  withAllRolesInText,
  withBasicFields,
  withBasicFieldsInText,
  withCodexApiKey,
  withCodexConfigToml,
  withGeminiEnv,
  withGrokConfigToml,
  withMetaTemplateValues,
  withRoleModelInText,
  withRoleNameInText,
  withRoleOneMInText,
} from "@/features/providers/derive"
import {
  bucketFetchModelsError,
  presetModelsUrl,
} from "@/features/providers/model-fetch"
import {
  PROVIDER_PRESETS,
  type ProviderPreset,
  presetsForApp,
} from "@/features/providers/presets"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { toStructuredError } from "@/lib/error"
import { parseJsonObject } from "@/lib/json"
import { cn } from "@/lib/utils"

import type { App, Provider } from "@/types/generated/bindings"

export function ProviderFormSheet({
  open,
  onOpenChange,
  editing,
  app,
  onSaved,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The provider being edited, or null for a new one. */
  editing: Provider | null
  /** 渲染哪一套字段：编辑已有供应商时用其自身的 app，否则取本参数，再否则
   *  claude（原有池）。 */
  app?: App
  onSaved: () => void
}) {
  const { t } = useTranslation()
  // effective app：编辑态供应商的 app 优先，其次调用方传入的新建 app，最后 claude。
  const effectiveApp: App = editing?.app ?? app ?? "claude"
  // 预设选择内聚进 Sheet（原由 ProvidersView 经 prop 注入）：新建态下用户在
  // 左栏 PresetPicker 点选即预填，可连续切换覆盖；编辑态不挂 picker。每次
  // 打开新建清空，避免上次会话选过的预设带到本次新建。
  const [preset, setPreset] = useState<ProviderPreset | null>(null)
  useEffect(() => {
    if (open && !editing) setPreset(null)
  }, [open, editing])
  const base =
    editing ??
    (preset
      ? providerFromPreset(preset, effectiveApp)
      : emptyProvider(effectiveApp))
  const [name, setName] = useState(base.name)
  const [configText, setConfigText] = useState(base.settingsConfig)
  const [endpoint, setEndpoint] = useState(providerEndpoint(base))
  const [apiKey, setApiKey] = useState(providerApiKey(base))
  const [authField, setAuthField] = useState<AuthField>(
    configAuthField(base.settingsConfig),
  )
  const [templateValues, setTemplateValues] = useState<Record<string, string>>(
    metaTemplateValues(base.meta),
  )
  const [save, { isLoading: saving }] = useSaveProviderMutation()
  const [fetchModels, { isLoading: fetching }] = useFetchModelsMutation()
  const [fetchedModels, setFetchedModels] = useState<string[]>([])
  const runWithToast = useMutateWithToast()

  useEffect(() => {
    if (!open) return
    const b = editing
      ? editing
      : preset
        ? providerFromPreset(preset, effectiveApp)
        : emptyProvider(effectiveApp)
    // 模板变量输入框的值存于 meta：重编已保存的云厂商供应商时，快照里占位符
    // 已被物化，先把这些值还原回占位符，重编体验与从预设新建一致。
    const values = metaTemplateValues(b.meta)
    setName(b.name)
    setConfigText(
      Object.keys(values).length > 0
        ? restoreTemplatePlaceholders(b.settingsConfig, values)
        : b.settingsConfig,
    )
    setEndpoint(providerEndpoint(b))
    setApiKey(providerApiKey(b))
    setAuthField(configAuthField(b.settingsConfig))
    setTemplateValues(values)
    // 上次会话拉到的模型列表属于旧表单状态，重开时清掉。
    setFetchedModels([])
  }, [editing, preset, open, effectiveApp])

  // JSON → form fields: whenever the snapshot text changes (editor edit or a
  // field merge), the env-backed fields mirror it. A text that doesn't parse
  // to an object keeps the fields as-is — the linter flags the red line, and
  // clobbering the fields with an empty derive would hide a broken edit.
  // Codex / Gemini 的字段直接从 configText 派生渲染（无镜像 state），本同步
  // 只服务 Claude 专属字段。
  useEffect(() => {
    if (effectiveApp !== "claude") return
    const result = parseJsonObject(configText)
    if (!result.ok) return
    setEndpoint(configEndpoint(configText))
    setApiKey(configApiKey(configText))
    setAuthField(configAuthField(configText))
  }, [configText, effectiveApp])

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

  function onEndpointChange(value: string) {
    setEndpoint(value)
    // 端点变了，旧端点拉到的模型列表不再可靠，清空下拉。
    setFetchedModels([])
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) =>
        withBasicFieldsInText(prev, { endpoint: value, apiKey, authField }),
      )
    }
  }

  function onApiKeyChange(value: string) {
    setApiKey(value)
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) =>
        withBasicFieldsInText(prev, { endpoint, apiKey: value, authField }),
      )
    }
  }

  function onRoleModelChange(role: ModelRoleId, value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withRoleModelInText(prev, role, value))
    }
  }

  function onRoleNameChange(role: ModelRoleId, value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withRoleNameInText(prev, role, value))
    }
  }

  function onRoleOneMChange(role: ModelRoleId, oneM: boolean) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withRoleOneMInText(prev, role, oneM))
    }
  }

  function onApplyAll() {
    if (!parseJsonObject(configText).ok) return
    const next = withAllRolesFromFirstInText(configText)
    if (next === null) return
    setConfigText(next)
    toast.success(t("providers.toast.applyAllSuccess"))
  }

  /** 调 fetchModels mutation 并处理结果（错误分桶 toast、成功填充
   *  fetchedModels）——Claude / Gemini 两条路径同一套错误标签契约，共用这一
   *  份错误渲染，避免分叉漂移。调用方负责各自的前置校验（端点 / key 是否
   *  必填）与参数构造。 */
  async function runFetchModels(args: {
    app: App
    baseUrl: string
    apiKey: string
    modelsUrl: string | null
  }): Promise<void> {
    const result = await fetchModels(args)
    if (result.error) {
      // RTK unions SerializedError in for internal failures; the repo seam
      // `toStructuredError` reduces either shape to its raw message string.
      const structured = toStructuredError(result.error)
      const message =
        structured?.kind === "app"
          ? structured.data
          : (structured?.message ?? String(result.error))
      const { kind, detail } = bucketFetchModelsError(message)
      toast.error(t(`providers.toast.fetchModels.${kind}`), {
        description: detail,
      })
      return
    }
    setFetchedModels(result.data)
    if (result.data.length === 0) {
      toast.warning(t("providers.toast.fetchModels.empty"))
    } else {
      toast.success(
        t("providers.toast.fetchModels.fetched", { count: result.data.length }),
      )
    }
  }

  /** 拉当前 Claude 供应商的模型列表：端点与 key 缺任一 → 对应提示；失败按
   *  后端错误串分桶提示（认证失败 / 端点未开放 / 超时 / 格式不支持 / 兜底）。 */
  async function onFetchModels() {
    if (!endpoint.trim()) {
      toast.error(t("providers.toast.fetchModels.endpointRequired"))
      return
    }
    if (!apiKey.trim()) {
      toast.error(t("providers.toast.fetchModels.keyRequired"))
      return
    }
    await runFetchModels({
      app: "claude",
      baseUrl: endpoint.trim(),
      apiKey: apiKey.trim(),
      // 端点等于某预设默认值时，带上该预设声明的 modelsUrl 覆写（如火山
      // /api/compatible 拼不出正确候选，必须精确指路）。
      modelsUrl: presetModelsUrl(endpoint, PROVIDER_PRESETS),
    })
  }

  /** 拉 Gemini 模型列表：key 缺失 → 提示；端点可空（后端 gemini_models_url
   *  处理空→默认 generativelanguage 端点）。失败分桶与 Claude 同一标签契约。 */
  async function onFetchGeminiModels() {
    const key = geminiApiKey(configText).trim()
    if (!key) {
      toast.error(t("providers.toast.fetchModels.keyRequired"))
      return
    }
    await runFetchModels({
      app: "gemini",
      baseUrl: geminiBaseUrl(configText).trim(),
      apiKey: key,
      // Gemini 端点形状固定（GET /v1beta/models），不走 modelsUrl 覆写。
      modelsUrl: null,
    })
  }

  /** 拉 OpenCode 模型列表：端点 = options.baseURL、认证 = options.apiKey（OpenAI
   *  兼容分支——后端对非 Gemini app 统一走 fetch_models）。端点 / key 缺任一 → 提示。 */
  async function onFetchOpenCodeModels() {
    const baseUrl = openCodeBaseUrl(configText).trim()
    const key = openCodeApiKey(configText).trim()
    if (!baseUrl) {
      toast.error(t("providers.toast.fetchModels.endpointRequired"))
      return
    }
    if (!key) {
      toast.error(t("providers.toast.fetchModels.keyRequired"))
      return
    }
    await runFetchModels({
      app: "opencode",
      baseUrl,
      apiKey: key,
      modelsUrl: null,
    })
  }

  /** Claude 下拉选中一个模型 → 回填五个角色（与「一键设置」同一写入引擎）。 */
  function onPickModel(model: string) {
    if (!parseJsonObject(configText).ok) return
    setConfigText((prev) => withAllRolesInText(prev, model))
    toast.success(t("providers.toast.applyAllSuccess"))
  }

  /** Gemini 下拉选中一个模型 → 写入 GEMINI_MODEL（Gemini 只有一个模型字段）。 */
  function onPickGeminiModel(model: string) {
    if (!parseJsonObject(configText).ok) return
    setConfigText((prev) => withGeminiEnv(prev, { GEMINI_MODEL: model }))
    toast.success(t("providers.toast.fetchModels.modelSet"))
  }

  function onAuthFieldChange(to: AuthField) {
    if (to === authField) return
    setAuthField(to)
    if (parseJsonObject(configText).ok) {
      // 值搬到新键、旧键删除——切换不丢 key 也不留双拼写。
      setConfigText((prev) => switchAuthField(prev, authField, to))
    }
  }

  function onTemplateVarChange(name: string, value: string) {
    const next = { ...templateValues, [name]: value }
    setTemplateValues(next)
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => replaceTemplateVarsInText(prev, next))
    }
  }

  // Codex / Gemini 字段直写 configText（与 Claude 的 onEndpointChange 同一模式：
  // 仅当外层 settingsConfig JSON 合法时回写，半截 JSON 不会被吞）。这两类应用无
  // 镜像 state——输入框直接读 derive 函数，写回经 derive 写入 configText。
  function onCodexApiKeyChange(value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withCodexApiKey(prev, value))
    }
  }

  function onCodexConfigChange(value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withCodexConfigToml(prev, value))
    }
  }

  function onGrokConfigChange(value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withGrokConfigToml(prev, value))
    }
  }

  function onGeminiApiKeyChange(value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withGeminiEnv(prev, { GEMINI_API_KEY: value }))
    }
  }

  function onGeminiModelChange(value: string) {
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) => withGeminiEnv(prev, { GEMINI_MODEL: value }))
    }
  }

  function onGeminiBaseUrlChange(value: string) {
    // 端点变了，旧端点拉到的模型列表不再可靠，清空下拉（与 Claude 的
    // onEndpointChange 同一处理）。
    setFetchedModels([])
    if (parseJsonObject(configText).ok) {
      setConfigText((prev) =>
        withGeminiEnv(prev, { GOOGLE_GEMINI_BASE_URL: value }),
      )
    }
  }

  async function onSave() {
    if (!name.trim()) {
      toast.error(t("providers.toast.nameRequired"))
      return
    }
    const snapshot = parseJsonObject(configText)
    if (!snapshot.ok) {
      toast.error(t("providers.toast.invalidConfig"), {
        description: snapshot.error,
      })
      return
    }
    // Codex / Gemini：字段已直写 configText，configText 即真相源，直接持久化。
    if (effectiveApp !== "claude") {
      const next = {
        ...base,
        settingsConfig: configText,
        app: effectiveApp,
        name: name.trim(),
      }
      const ok = await runWithToast(save, next, {
        success: { key: "providers.toast.saved", vars: { name: name.trim() } },
        failed: { key: "providers.toast.saveFailed" },
      })
      if (ok) onSaved()
      return
    }
    // 把模板变量物化进快照；仍残留 `${VAR}` 说明有变量未填——拒绝保存，避免
    // 切换时把字面量占位符写进 live 配置。
    const materialized = replaceTemplateVarsInText(configText, templateValues)
    const unfilled = extractTemplateVars(materialized)
    if (unfilled.length > 0) {
      toast.error(
        t("providers.toast.unfilledTemplateVars", {
          vars: unfilled.map((v) => `\${${v}}`).join(", "),
        }),
      )
      return
    }
    // Rebuild the snapshot from the working JSON text (preserving everything
    // the form doesn't own), then attach the edited name and ship the upsert.
    // The endpoint is trimmed here, not on every keystroke, so typing an
    // in-progress value (trailing spaces mid-edit) isn't fought by the input.
    // The template values ride along in the app-side meta so a later edit can
    // pre-fill the inputs (the live file never sees meta).
    const next = withBasicFields(
      {
        ...base,
        settingsConfig: materialized,
        meta: withMetaTemplateValues(base.meta, templateValues),
      },
      { endpoint: endpoint.trim(), apiKey, authField },
    )
    const ok = await runWithToast(
      save,
      { ...next, name: name.trim() },
      {
        success: { key: "providers.toast.saved", vars: { name: name.trim() } },
        failed: { key: "providers.toast.saveFailed" },
      },
    )
    if (ok) onSaved()
  }

  // 模板变量输入区：快照文本里出现的 `${VAR}` 占位符 + meta 里记录的已填值
  // （重编时占位符已物化，输入框仍要显示）。无模板变量的普通供应商整段隐藏。
  const templateVarNames = Array.from(
    new Set([
      ...extractTemplateVars(configText),
      ...Object.keys(templateValues),
    ]),
  )
  // key 输入区显示条件：快照带模板变量（云厂商的认证走模板变量）或分类为官方
  // /云厂商预设时隐藏，普通供应商照常显示。草稿分类恒为 custom，故官方预设须
  // 看 preset 自身的分类。
  const category = preset?.category ?? base.category
  const showKeyFields =
    templateVarNames.length === 0 &&
    category !== "official" &&
    category !== "cloud_provider"
  // 双栏左栏（预设选择器）只在新建态 + 该 app 有内置预设时挂载：编辑已有
  // 供应商不挂；opencode 附加模式 presetsForApp 返回空也不挂。
  const showPicker = !editing && presetsForApp(effectiveApp).length > 0

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      {/* 宽度：双栏（新建 + 有内置预设）80vw 容下左预设栏与右表单；单栏 60vw。
          p-0 让左栏贴边、表单与头/脚各自 px-6 自管 padding（双栏时左栏 border-r
          分隔）。 */}
      <SheetContent
        className={cn(
          "sm:max-w-none p-0",
          showPicker ? "w-[80vw]" : "w-[60vw]",
        )}
      >
        <SheetHeader className="px-6 pt-6">
          <SheetTitle>
            {editing
              ? t("providers.form.editTitle")
              : preset
                ? t("providers.form.presetTitle")
                : t("providers.form.newTitle")}
          </SheetTitle>
          {preset ? (
            <p className="text-muted-foreground text-xs">
              {t("providers.form.presetHint", { name: preset.name })}
            </p>
          ) : null}
        </SheetHeader>

        <div className="flex min-h-0 flex-1">
          {showPicker ? (
            <PresetPicker app={effectiveApp} onSelect={setPreset} />
          ) : null}
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6">
            {effectiveApp === "claude" && templateVarNames.length > 0 ? (
              <div className="rounded-lg border bg-muted/40 p-3">
                <p className="mb-1 text-xs font-medium">
                  {t("providers.form.templateVars")}
                </p>
                <p className="text-muted-foreground mb-2 text-xs">
                  {t("providers.form.templateVarsHint")}
                </p>
                <div className="flex flex-col gap-2">
                  {templateVarNames.map((name) => (
                    <Field key={name} label={name}>
                      <Input
                        value={templateValues[name] ?? ""}
                        onChange={(e) =>
                          onTemplateVarChange(name, e.target.value)
                        }
                        spellCheck={false}
                      />
                    </Field>
                  ))}
                </div>
              </div>
            ) : null}
            <SectionHeader>{t("providers.form.section.basic")}</SectionHeader>
            <Field label={t("providers.form.name")}>
              <Input
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("providers.form.namePlaceholder")}
              />
            </Field>
            {effectiveApp === "claude" ? (
              <Field label={t("providers.form.endpoint")}>
                <Input
                  value={endpoint}
                  onChange={(e) => onEndpointChange(e.target.value)}
                  placeholder="https://api.example.com"
                  spellCheck={false}
                />
              </Field>
            ) : null}
            {effectiveApp === "claude" && showKeyFields ? (
              <div className="flex items-end gap-2">
                <Field
                  label={t("providers.form.authField")}
                  className="shrink-0"
                >
                  <Select
                    value={authField}
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
                <Field
                  label={t("providers.form.apiKey")}
                  className="min-w-0 flex-1"
                >
                  <Input
                    type="password"
                    value={apiKey}
                    onChange={(e) => onApiKeyChange(e.target.value)}
                    placeholder={t("providers.form.apiKeyPlaceholder")}
                    spellCheck={false}
                  />
                </Field>
              </div>
            ) : null}
            {effectiveApp === "codex" ? (
              <>
                <Field label={t("providers.form.apiKey")}>
                  <Input
                    type="password"
                    value={codexApiKey(configText)}
                    onChange={(e) => onCodexApiKeyChange(e.target.value)}
                    placeholder={t("providers.form.apiKeyPlaceholder")}
                    spellCheck={false}
                  />
                </Field>
                <Field label={t("providers.form.codexConfig")}>
                  <Textarea
                    value={codexConfigToml(configText)}
                    onChange={(e) => onCodexConfigChange(e.target.value)}
                    rows={12}
                    spellCheck={false}
                    className="font-mono text-xs"
                  />
                  <p className="text-muted-foreground text-xs">
                    {t("providers.form.codexConfigHint")}
                  </p>
                </Field>
              </>
            ) : null}
            {effectiveApp === "gemini" ? (
              <>
                <Field label={t("providers.form.apiKey")}>
                  <Input
                    type="password"
                    value={geminiApiKey(configText)}
                    onChange={(e) => onGeminiApiKeyChange(e.target.value)}
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
                      onClick={onFetchGeminiModels}
                      disabled={fetching}
                    >
                      <RefreshCw
                        className={cn("size-3.5", fetching && "animate-spin")}
                      />
                      {fetching
                        ? t("providers.form.fetchModels.fetching")
                        : t("providers.form.fetchModels.fetch")}
                    </Button>
                  </div>
                  <Input
                    value={geminiModel(configText)}
                    onChange={(e) => onGeminiModelChange(e.target.value)}
                    spellCheck={false}
                  />
                  {fetchedModels.length > 0 ? (
                    <Select
                      onValueChange={(model) => {
                        if (typeof model === "string") onPickGeminiModel(model)
                      }}
                    >
                      <SelectTrigger
                        className="font-mono text-xs"
                        aria-label={t(
                          "providers.form.fetchModels.geminiPlaceholder",
                        )}
                      >
                        <SelectValue
                          placeholder={t(
                            "providers.form.fetchModels.geminiPlaceholder",
                          )}
                        />
                      </SelectTrigger>
                      <SelectContent>
                        {fetchedModels.map((model) => (
                          <SelectItem
                            key={model}
                            value={model}
                            className="font-mono text-xs"
                          >
                            {model}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  ) : null}
                </div>
                <Field label={t("providers.form.geminiBaseUrl")}>
                  <Input
                    value={geminiBaseUrl(configText)}
                    onChange={(e) => onGeminiBaseUrlChange(e.target.value)}
                    placeholder="https://generativelanguage.googleapis.com"
                    spellCheck={false}
                  />
                </Field>
              </>
            ) : null}
            {effectiveApp === "grok" ? (
              <Field label={t("providers.form.grokConfig")}>
                <Textarea
                  value={grokConfigToml(configText)}
                  onChange={(e) => onGrokConfigChange(e.target.value)}
                  rows={12}
                  spellCheck={false}
                  className="font-mono text-xs"
                />
                <p className="text-muted-foreground text-xs">
                  {t("providers.form.grokConfigHint")}
                </p>
              </Field>
            ) : null}
            {effectiveApp === "opencode" ? (
              <OpenCodeFormFields
                configText={configText}
                onChange={setConfigText}
                fetching={fetching}
                fetchedModels={fetchedModels}
                onFetchModels={onFetchOpenCodeModels}
              />
            ) : null}
            {effectiveApp === "claude" ? (
              <div className="rounded-md border p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <Label className="text-muted-foreground text-xs">
                    {t("providers.form.modelMapping")}
                  </Label>
                  <div className="flex items-center gap-2">
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
                </div>
                <p className="mt-1.5 mb-2 text-xs text-muted-foreground">
                  {t("providers.form.modelMappingHint")}
                </p>
                {fetchedModels.length > 0 ? (
                  <div className="mb-2">
                    <Select
                      onValueChange={(model) => {
                        if (typeof model === "string") onPickModel(model)
                      }}
                    >
                      <SelectTrigger
                        className="font-mono text-xs"
                        aria-label={t("providers.form.fetchModels.placeholder")}
                      >
                        <SelectValue
                          placeholder={t(
                            "providers.form.fetchModels.placeholder",
                          )}
                        />
                      </SelectTrigger>
                      <SelectContent>
                        {fetchedModels.map((model) => (
                          <SelectItem
                            key={model}
                            value={model}
                            className="font-mono text-xs"
                          >
                            {model}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}
                <div className="space-y-2">
                  {roleRows.map(({ role, fields }) => (
                    <div key={role.id} className="space-y-1">
                      <div className="flex items-center justify-between">
                        <Label className="text-muted-foreground text-xs">
                          {t(`providers.form.role.${role.id}`)}
                        </Label>
                        {role.supportsOneM ? (
                          <label
                            htmlFor={`model-role-one-m-${role.id}`}
                            className="flex cursor-pointer items-center gap-1.5 text-xs text-muted-foreground"
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
                      <div className="grid grid-cols-2 gap-2">
                        <Field label={t("providers.form.displayName")}>
                          <Input
                            value={fields.name}
                            onChange={(e) =>
                              onRoleNameChange(role.id, e.target.value)
                            }
                            placeholder={stripOneM(fields.model)}
                            spellCheck={false}
                          />
                        </Field>
                        <Field label={t("providers.form.requestModel")}>
                          <Input
                            value={fields.model}
                            onChange={(e) =>
                              onRoleModelChange(role.id, e.target.value)
                            }
                            spellCheck={false}
                          />
                        </Field>
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            ) : null}
            <SectionHeader>
              {t("providers.form.section.advanced")}
            </SectionHeader>
            <Field label={t("providers.form.settingsJson")}>
              <JsonEditor
                value={configText}
                onChange={setConfigText}
                placeholder={t("providers.form.settingsJsonPlaceholder")}
                className="h-72"
              />
            </Field>
          </div>
        </div>

        <SheetFooter className="px-6 pb-6">
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button disabled={saving} onClick={onSave}>
            {saving ? t("common.saving") : t("common.save")}
          </Button>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  )
}

// 表单章节标题：上方 hairline + 小标题，给长表单（基本信息 / 认证 / 模型映射
// / 高级配置）可扫读的视觉锚点。border-t 兼作段落分隔。
function Field({
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
