// Provider editor as a Sheet (side panel) — new and edit both flow through
// here. The sheet owns the shared skeleton: the name field (BasicSection), the
// fetch-model plumbing (runFetchModels + per-app arg extraction) that every
// app's field section shares, and the settings JSON editor at the bottom.
// Each app's own fields render through a per-app partition: claude →
// ClaudeFormFields (template vars / endpoint / auth / model mapping),
// opencode → OpenCodeFormFields; codex / gemini / grok stay inline below
// (their blocks are small, the seam is the same).
//
// configText (the settingsConfig JSON text) is the single source of truth:
// every field reads straight from it via the codec derive functions — there
// is no mirrored field state to keep in sync — and every write goes through
// `guardedWrite`, which refuses to merge a field edit back while the JSON text
// does not parse to an object (half-broken JSON is never swallowed: the
// editor's in-progress edit survives until it parses again). The round-trip
// rules live in the codecs (codecs/claude.ts etc.) as pure functions; the
// guard itself is lib/json's `guardedRewrite`. Draft seeding on open and
// save-time finalization go through the per-app ports in codecs/draft — the
// sheet itself carries no app-specific step.

import { RefreshCw } from "lucide-react"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useFetchModelsMutation,
  useSaveProviderMutation,
} from "@/app/store/api"
import { JsonEditor } from "@/components/json-editor"
import { SectionHeader } from "@/components/section-header"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Sheet,
  SheetContent,
  SheetFooter,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet"
import { Textarea } from "@/components/ui/textarea"
// 草稿种子 / 保存收敛端口（codecs/draft）：按 app 分派的纯函数,骨架不内联
// 任何 app 特殊步骤——不进 derive 聚合重导出（保持 draft → derive 单向）。
import { finalizeDraft, seedDraftText } from "@/features/providers/codecs/draft"
import { ClaudeFormFields } from "@/features/providers/components/claude-form-fields"
import {
  BasicSection,
  Field,
} from "@/features/providers/components/form-fields"
import { ModelPickSelect } from "@/features/providers/components/model-pick-select"
import { OpenCodeFormFields } from "@/features/providers/components/opencode-form-fields"
import { PresetPicker } from "@/features/providers/components/preset-picker"
import {
  codexApiKey,
  codexConfigToml,
  configApiKey,
  configEndpoint,
  emptyProvider,
  geminiApiKey,
  geminiBaseUrl,
  geminiModel,
  grokConfigToml,
  metaTemplateValues,
  openCodeApiKey,
  openCodeBaseUrl,
  providerFromPreset,
  providerLiveManaged,
  restoreTemplatePlaceholders,
  withCodexApiKey,
  withCodexConfigToml,
  withGeminiEnv,
  withGrokConfigToml,
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
import { rawErrorText } from "@/lib/error"
import { guardedRewrite, parseJsonObject } from "@/lib/json"
import { cn } from "@/lib/utils"

import type { App, Provider } from "@/types/generated/bindings"

export function ProviderFormSheet({
  open,
  onOpenChange,
  editing,
  app,
  onSaved,
  onResetEditing,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The provider being edited, or null for a new one. */
  editing: Provider | null
  /** 渲染哪一套字段：编辑已有供应商时用其自身的 app，否则取本参数，再否则
   *  claude（原有池）。 */
  app?: App
  onSaved: () => void
  /** 复制态（editing 是 id 空的副本）下点选预设 = 放弃副本改用预设草稿：
   *  通知父组件清掉 editing。 */
  onResetEditing: () => void
}) {
  const { t } = useTranslation()
  // effective app：编辑态供应商的 app 优先，其次调用方传入的新建 app，最后 claude。
  const effectiveApp: App = editing?.app ?? app ?? "claude"
  // 预设选择内聚进 Sheet（原由 ProvidersView 经 prop 注入）：新建 / 复制态
  // 下用户在左栏 PresetPicker 点选即预填，可连续切换覆盖；编辑已有供应商
  // 不挂 picker。每次打开都清空选中，避免上次会话选过的预设带过来。
  const [preset, setPreset] = useState<ProviderPreset | null>(null)
  useEffect(() => {
    if (open) setPreset(null)
  }, [open])
  const base =
    editing ??
    (preset
      ? providerFromPreset(preset, effectiveApp)
      : emptyProvider(effectiveApp))
  const [name, setName] = useState(base.name)
  const [configText, setConfigText] = useState(base.settingsConfig)
  const [templateValues, setTemplateValues] = useState<Record<string, string>>(
    metaTemplateValues(base.meta),
  )
  // 自动应用开关（模型区）：开着时编辑任一角色模型自动同步全部角色。默认值
  // 在 open effect 里按新建/复制 vs 编辑设置。
  const [autoSync, setAutoSync] = useState(false)
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
    // 编辑/复制保留既有配置原样；真·新建的种子策略按 app 分派到
    // seedDraftText（claude 默认 [1M]，其余 app 恒等——"{}" 留成 "{}"）。
    const baseText =
      Object.keys(values).length > 0
        ? restoreTemplatePlaceholders(b.settingsConfig, values)
        : b.settingsConfig
    setConfigText(editing ? baseText : seedDraftText(effectiveApp, baseText))
    setTemplateValues(values)
    // 自动应用开关：新建/复制默认开（编辑任一模型同步全部角色），编辑已有
    // 供应商默认关（不动用户配置）。
    setAutoSync(!editing?.id)
    // 上次会话拉到的模型列表属于旧表单状态，重开时清掉。
    setFetchedModels([])
  }, [editing, preset, open, effectiveApp])

  /** configText 为真相源 + 写回收口：仅当外层 settingsConfig JSON 合法时回写
   *  （半截 JSON 不会被吞——守卫在 lib/json 的 guardedRewrite，可测）——所有
   *  字段写回经此单一归属，handler 不再各自重复 parse 守卫。返回是否真的写
   *  了（调用方据此决定 toast 等副作用）。 */
  function guardedWrite(update: (prev: string) => string): boolean {
    const next = guardedRewrite(configText, update)
    if (next === null) return false
    setConfigText(next)
    return true
  }

  /** 一次 fetch_models 调用的完整参数（app + 端点 + 认证 + modelsUrl 覆写）。
   *  per-app 提取见 fetchModelsArgsFor。 */
  type FetchModelsArgs = {
    app: App
    baseUrl: string
    apiKey: string
    modelsUrl: string | null
  }

  /** 调 fetchModels mutation 并处理结果（错误分桶 toast、成功填充
   *  fetchedModels）——Claude / Gemini 两条路径同一套错误标签契约，共用这一
   *  份错误渲染，避免分叉漂移。调用方负责各自的前置校验（端点 / key 是否
   *  必填）与参数构造。 */
  async function runFetchModels(args: FetchModelsArgs): Promise<void> {
    const result = await fetchModels(args)
    if (result.error) {
      // RTK unions SerializedError in for internal failures; `rawErrorText`
      // reduces either shape to the backend-visible text (AppError data first).
      const message = rawErrorText(result.error)
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

  /** 各 app 拉模型列表的参数提取（per-app 小表——三份 fetch 处理器的差异只
   *  在这一层：参数来源、前置校验、modelsUrl 覆写；错误分桶与成功填充共用
   *  runFetchModels）。判别联合：`ok: true` 时 args 必存在、`ok: false` 时
   *  missing 给出缺的部分（endpoint / key，调用方提示对应文案）——互斥
   *  不变量在类型里，不用 `!`。codex / grok 无 fetch 入口，不在此表。 */
  function fetchModelsArgsFor(
    app: "claude" | "gemini" | "opencode",
  ):
    | { ok: true; args: FetchModelsArgs }
    | { ok: false; missing: "endpoint" | "key" } {
    switch (app) {
      case "claude": {
        const baseUrl = configEndpoint(configText).trim()
        const key = configApiKey(configText).trim()
        if (!baseUrl) return { ok: false, missing: "endpoint" }
        if (!key) return { ok: false, missing: "key" }
        return {
          ok: true,
          args: {
            app,
            baseUrl,
            apiKey: key,
            // 端点等于某预设默认值时，带上该预设声明的 modelsUrl 覆写（如火山
            // /api/compatible 拼不出正确候选，必须精确指路）。
            modelsUrl: presetModelsUrl(baseUrl, PROVIDER_PRESETS),
          },
        }
      }
      case "gemini": {
        const key = geminiApiKey(configText).trim()
        if (!key) return { ok: false, missing: "key" }
        // Gemini 端点形状固定（GET /v1beta/models），不走 modelsUrl 覆写；
        // 端点可空（后端 gemini_models_url 处理空→默认 generativelanguage 端点）。
        return {
          ok: true,
          args: {
            app,
            baseUrl: geminiBaseUrl(configText).trim(),
            apiKey: key,
            modelsUrl: null,
          },
        }
      }
      case "opencode": {
        const baseUrl = openCodeBaseUrl(configText).trim()
        const key = openCodeApiKey(configText).trim()
        if (!baseUrl) return { ok: false, missing: "endpoint" }
        if (!key) return { ok: false, missing: "key" }
        return {
          ok: true,
          args: { app, baseUrl, apiKey: key, modelsUrl: null },
        }
      }
    }
  }

  /** 拉当前供应商的模型列表（统一入口）：参数提取与前置校验按 app 分表
   *  （fetchModelsArgsFor），失败按后端错误串分桶提示（认证失败 / 端点未开放 /
   *  超时 / 格式不支持 / 兜底）。 */
  async function onFetchModelsFor(app: "claude" | "gemini" | "opencode") {
    const result = fetchModelsArgsFor(app)
    if (!result.ok) {
      toast.error(
        t(
          result.missing === "endpoint"
            ? "providers.toast.fetchModels.endpointRequired"
            : "providers.toast.fetchModels.keyRequired",
        ),
      )
      return
    }
    await runFetchModels(result.args)
  }

  /** Claude 区拉模型列表（OpenAI 兼容分支的后端入口是 fetch_models）。 */
  function onFetchModels() {
    void onFetchModelsFor("claude")
  }

  /** Gemini 区拉模型列表。 */
  function onFetchGeminiModels() {
    void onFetchModelsFor("gemini")
  }

  /** OpenCode 区拉模型列表（端点 = options.baseURL、认证 = options.apiKey）。 */
  function onFetchOpenCodeModels() {
    void onFetchModelsFor("opencode")
  }

  /** Gemini 下拉选中一个模型 → 写入 GEMINI_MODEL（Gemini 只有一个模型字段）。 */
  function onPickGeminiModel(model: string) {
    if (guardedWrite((prev) => withGeminiEnv(prev, { GEMINI_MODEL: model }))) {
      toast.success(t("providers.toast.fetchModels.modelSet"))
    }
  }

  function onTemplateVarChange(name: string, value: string) {
    // 只更新内存值，不实时物化 configText：物化会让占位符从 configText 消失，
    // templateVarNames（extractTemplateVars(configText) 的投影）随之缩水/重排
    // ——输入框跳位，半截值还会污染快照。物化只在保存时做
    // （replaceTemplateVarsInText + 残留校验）。
    setTemplateValues((prev) => ({ ...prev, [name]: value }))
  }

  // Codex / Gemini / Grok 字段直写 configText（与 claude 分区同一模式：仅当
  // 外层 settingsConfig JSON 合法时回写，半截 JSON 不会被吞——守卫经
  // guardedWrite 单一归属）。这几类应用无镜像 state——输入框直接读 derive
  // 函数，写回经 derive 写入 configText。
  function onCodexApiKeyChange(value: string) {
    guardedWrite((prev) => withCodexApiKey(prev, value))
  }

  function onCodexConfigChange(value: string) {
    guardedWrite((prev) => withCodexConfigToml(prev, value))
  }

  function onGrokConfigChange(value: string) {
    guardedWrite((prev) => withGrokConfigToml(prev, value))
  }

  function onGeminiApiKeyChange(value: string) {
    guardedWrite((prev) => withGeminiEnv(prev, { GEMINI_API_KEY: value }))
  }

  function onGeminiModelChange(value: string) {
    guardedWrite((prev) => withGeminiEnv(prev, { GEMINI_MODEL: value }))
  }

  function onGeminiBaseUrlChange(value: string) {
    // 端点变了，旧端点拉到的模型列表不再可靠，清空下拉（与 claude 分区的
    // onEndpointChange 同一处理）。
    setFetchedModels([])
    guardedWrite((prev) =>
      withGeminiEnv(prev, { GOOGLE_GEMINI_BASE_URL: value }),
    )
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
    // 按 app 收敛保存结果（finalizeDraft）：claude 物化模板变量并校验残留、
    // 归一基础字段、meta 记录变量值；其余 app 的 configText 即真相源，原样
    // 持久化。返回不 ok（有 `${VAR}` 未填值）则拒绝保存——字面量占位符绝不
    // 写进快照/live 配置（后端切换时同样拦截）。
    const finalized = finalizeDraft(
      effectiveApp,
      configText,
      templateValues,
      base.meta,
    )
    if (!finalized.ok) {
      toast.error(
        t("providers.toast.unfilledTemplateVars", {
          vars: finalized.unfilled.map((v) => `\${${v}}`).join(", "),
        }),
      )
      return
    }
    const next = {
      ...base,
      settingsConfig: finalized.settingsConfig,
      meta: finalized.meta,
      app: effectiveApp,
      name: name.trim(),
    }
    const ok = await runWithToast(save, next, {
      success: { key: "providers.toast.saved", vars: { name: name.trim() } },
      failed: { key: "providers.toast.saveFailed" },
    })
    if (ok) onSaved()
  }

  // 双栏左栏（预设选择器）只在新建 / 复制态 + 该 app 有内置预设时挂载：编辑
  // 已有供应商（id 非空）不挂；opencode 附加模式 presetsForApp 返回空也不挂。
  // 复制态点选预设 = 用预设草稿替换副本（base 优先级：editing 先于 preset，
  // 故 onSelect 里把副本 editing 清掉，见下）。
  const showPicker =
    (!editing || editing.id === "") && presetsForApp(effectiveApp).length > 0

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      {/* 弹层面 = --popover（亮 #fff / 暗 #26262a 中灰）——暗色三档阶梯里
          popover 独占「弹层浮起位」，与页面 #1c1c1e 立得开；亮色纯白浮在
          页面浅渐变上。不用 bg-app：那是整窗画布面（页面灰 + 紫→蓝渐变），
          弹层带渐变显脏、暗色下与页面同色浮不起来（用户决策 2026-08-14
          重调）。宽度用 min() 收敛：双栏（新建 + 有内置预设）72rem 封顶
          防大窗失控，单栏 42rem；窄窗下 84vw 防挤压。p-0 让左栏贴边、
          表单与头/脚各自 px-6 自管 padding。 */}
      <SheetContent
        className={cn(
          "sm:max-w-none bg-popover p-0",
          showPicker ? "w-[min(84vw,72rem)]" : "w-[min(84vw,42rem)]",
        )}
      >
        <SheetHeader className="px-6 pt-6">
          <div className="flex items-center gap-2">
            <SheetTitle>
              {/* 复制走 editing 通道但 id 清空（新建语义）：id 非空才是编辑。 */}
              {editing?.id
                ? t("providers.form.editTitle")
                : preset
                  ? t("providers.form.presetTitle")
                  : t("providers.form.newTitle")}
            </SheetTitle>
            {/* 当前编辑的应用池徽标——长表单里一眼知道在配哪个 app 的供应商。 */}
            <Badge
              variant="secondary"
              className="h-5 shrink-0 px-1.5 text-[11px] font-normal"
            >
              {t(`providers.app.${effectiveApp}`)}
            </Badge>
            {/* 复制草稿（id 空）未加入 live，不显示「已启用」徽标。 */}
            {editing?.id &&
            effectiveApp === "opencode" &&
            providerLiveManaged(editing) ? (
              <Badge
                variant="outline"
                className="h-5 shrink-0 px-1.5 text-[11px] font-normal"
              >
                {t("providers.live.enabled")}
              </Badge>
            ) : null}
          </div>
          {preset ? (
            <p className="text-muted-foreground text-xs">
              {t("providers.form.presetHint", { name: preset.name })}
            </p>
          ) : null}
        </SheetHeader>

        {/* 双栏单面结构：左预设栏与右表单共享弹层面（--popover），中间
            border-r hairline 分隔——不再各自成卡（暗色下 card #0d0d0f 比
            弹层更深，卡片呈凹洞状；亮色下卡在渐变上靠阴影勉强浮）。减线
            语言：线条只做结构分隔，控件边框才是边框。 */}
        <div className="flex min-h-0 flex-1">
          {showPicker ? (
            <PresetPicker
              app={effectiveApp}
              selected={preset}
              onSelect={(p) => {
                // 复制态（editing 是 id 空的副本）点预设 = 放弃副本改用预设
                // 草稿——base 计算 editing 优先，必须把副本清掉才生效。
                onResetEditing()
                setPreset(p)
              }}
            />
          ) : null}
          {/* 表单流：px-6 与头/脚对齐，py-3 上下呼吸。不再有卡片面——
              分区靠 SectionHeader 小标题 + 间距，不靠盒子。 */}
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6 py-3">
            {effectiveApp === "claude" ? (
              <ClaudeFormFields
                configText={configText}
                onChange={guardedWrite}
                fetching={fetching}
                fetchedModels={fetchedModels}
                onFetchModels={onFetchModels}
                onEndpointEdited={() => setFetchedModels([])}
                name={name}
                onNameChange={setName}
                templateValues={templateValues}
                onTemplateVarChange={onTemplateVarChange}
                autoSync={autoSync}
                onAutoSyncChange={setAutoSync}
                category={base.category}
              />
            ) : (
              <>
                <BasicSection name={name} onNameChange={setName} />
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
                            className={cn(
                              "size-3.5",
                              fetching && "animate-spin",
                            )}
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
                      <ModelPickSelect
                        models={fetchedModels}
                        placeholder={t(
                          "providers.form.fetchModels.geminiPlaceholder",
                        )}
                        onPick={onPickGeminiModel}
                      />
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
                    onChange={(next) => guardedWrite(() => next)}
                    fetching={fetching}
                    fetchedModels={fetchedModels}
                    onFetchModels={onFetchOpenCodeModels}
                  />
                ) : null}
              </>
            )}
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
