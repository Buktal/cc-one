// OpenCode 供应商表单字段（独立组件）：npm 包下拉 + baseURL + apiKey + headers
// 键值编辑器 + models 编辑器 + 获取模型按钮。字段差异大——headers 键值编辑器与
// models map 编辑器是其它应用没有的 UI，塞进 ProviderFormSheet 会让它臃肿，故
// 独立成组件。
//
// 真相源是 configText（settingsConfig = opencode.json 的 `provider.<key>` 子树
// 内容）。npm / baseURL / apiKey 直写 configText（经 derive 的 withOpenCode*），
// 与 codex / gemini 的「字段直写 configText」同一模式。headers / models 因要增删
// 行、且空键行需保留到用户填完，维护本地行 state，再经 withOpenCode* 把非空行
// 写回 configText；外部 configText 变化（JSON 编辑器手改 / 预设切换）用
// lastEmitted ref 防回环地同步回本地行。fetch 候选由父组件注入（复用其错误分桶
// 的 runFetchModels，避免分叉），选中候选即追加为一行 model。

import { Plus, RefreshCw, Trash2 } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
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
  openCodeApiKey,
  openCodeBaseUrl,
  openCodeHeaders,
  openCodeModels,
  openCodeNpm,
  withOpenCodeApiKey,
  withOpenCodeBaseUrl,
  withOpenCodeHeaders,
  withOpenCodeModels,
  withOpenCodeNpm,
} from "@/features/providers/derive"
import { cn } from "@/lib/utils"

/** npm 包候选：OpenCode AI SDK 的 5 个包（spec §4.4），label 是人话描述，value 是
 *  写进 opencode.json 的完整包名。顺序把 openai-compatible 放首位——绝大多数第三方
 *  / 国内大厂端点用它。 */
const NPM_PACKAGES = [
  { value: "@ai-sdk/openai-compatible", label: "OpenAI Compatible" },
  { value: "@ai-sdk/openai", label: "OpenAI Responses" },
  { value: "@ai-sdk/anthropic", label: "Anthropic" },
  { value: "@ai-sdk/google", label: "Google (Gemini)" },
  { value: "@ai-sdk/amazon-bedrock", label: "Amazon Bedrock" },
] as const

/** headers 编辑器的一行（id 是稳定 React key，key/value 是编辑值——key 可临时为空）。 */
type HeaderRow = { id: number; key: string; value: string }
/** models 编辑器的一行（modelId 是 opencode.json 的 `models.<id>` 键）。 */
type ModelRow = { id: number; modelId: string; name: string }

/** 模块级行 id 序列：跨组件实例递增，保证行 React key 稳定唯一（增删 / 重排不晃）。 */
let rowSeq = 0
function nextRowId(): number {
  rowSeq += 1
  return rowSeq
}

export function OpenCodeFormFields({
  configText,
  onChange,
  fetching,
  fetchedModels,
  onFetchModels,
}: {
  configText: string
  onChange: (next: string) => void
  fetching: boolean
  fetchedModels: string[]
  onFetchModels: () => void
}) {
  const { t } = useTranslation()
  /** 上一次本组件写回的 configText——外部 configText 变化与之相等时跳过同步，
   *  避免回环（本地写回 → configText 变 → 又重置本地行 → 丢空键行）。 */
  const lastEmitted = useRef(configText)
  const [headerRows, setHeaderRows] = useState<HeaderRow[]>(() =>
    toHeaderRows(openCodeHeaders(configText)),
  )
  const [modelRows, setModelRows] = useState<ModelRow[]>(() =>
    toModelRows(openCodeModels(configText)),
  )

  useEffect(() => {
    if (configText === lastEmitted.current) return
    lastEmitted.current = configText
    setHeaderRows(toHeaderRows(openCodeHeaders(configText)))
    setModelRows(toModelRows(openCodeModels(configText)))
  }, [configText])

  /** 写回 configText 并记下本次发出的值（供 useEffect 防回环判定）。 */
  function emit(next: string) {
    lastEmitted.current = next
    onChange(next)
  }

  // npm / baseURL / apiKey：无增删行、无空值问题，直写 configText。
  function onNpmChange(npm: string) {
    emit(withOpenCodeNpm(configText, npm))
  }
  function onBaseUrlChange(baseURL: string) {
    emit(withOpenCodeBaseUrl(configText, baseURL))
  }
  function onApiKeyChange(apiKey: string) {
    emit(withOpenCodeApiKey(configText, apiKey))
  }

  // headers：更新本地行，再把「键非空」的行写回 configText（空键行留本地待填）。
  function commitHeaders(rows: HeaderRow[]) {
    const map: Record<string, string> = {}
    for (const r of rows) {
      const key = r.key.trim()
      if (key) map[key] = r.value
    }
    emit(withOpenCodeHeaders(configText, map))
  }
  function updateHeader(id: number, patch: Partial<HeaderRow>) {
    const rows = headerRows.map((r) => (r.id === id ? { ...r, ...patch } : r))
    setHeaderRows(rows)
    commitHeaders(rows)
  }
  function addHeader() {
    // 空 key 行不写回 configText（避免脏 map），只入本地行等用户填键。
    setHeaderRows((rows) => [...rows, { id: nextRowId(), key: "", value: "" }])
  }
  function removeHeader(id: number) {
    const rows = headerRows.filter((r) => r.id !== id)
    setHeaderRows(rows)
    commitHeaders(rows)
  }

  // models：与 headers 同一模式（空 modelId 行留本地待填）。
  function commitModels(rows: ModelRow[]) {
    const map: Record<string, { name?: string }> = {}
    for (const r of rows) {
      const modelId = r.modelId.trim()
      if (modelId) map[modelId] = r.name ? { name: r.name } : {}
    }
    emit(withOpenCodeModels(configText, map))
  }
  function updateModel(id: number, patch: Partial<ModelRow>) {
    const rows = modelRows.map((r) => (r.id === id ? { ...r, ...patch } : r))
    setModelRows(rows)
    commitModels(rows)
  }
  function addModel(modelId = "", name = "") {
    // 同步构造新行（基于当前 modelRows），setModelRows 与 commitModels 用同一份，
    // 避免 functional update 与闭包 commit 两路分叉（旧值 + 重复 id）。
    const newRows = [...modelRows, { id: nextRowId(), modelId, name }]
    setModelRows(newRows)
    if (modelId.trim()) commitModels(newRows)
  }
  function removeModel(id: number) {
    const rows = modelRows.filter((r) => r.id !== id)
    setModelRows(rows)
    commitModels(rows)
  }
  /** fetch 下拉选中一个候选 → 追加为一行 model（已存在同 id 则跳过）。 */
  function onPickModel(model: string) {
    if (modelRows.some((r) => r.modelId === model)) return
    addModel(model, "")
  }

  const npm = openCodeNpm(configText)
  const baseURL = openCodeBaseUrl(configText)
  const apiKey = openCodeApiKey(configText)

  return (
    <>
      <Field label={t("providers.form.openCodeNpm")}>
        <Select value={npm} onValueChange={(v) => v && onNpmChange(v)}>
          <SelectTrigger
            className="font-mono text-xs"
            aria-label={t("providers.form.openCodeNpm")}
          >
            <SelectValue
              placeholder={t("providers.form.openCodeNpmPlaceholder")}
            />
          </SelectTrigger>
          <SelectContent>
            {NPM_PACKAGES.map((pkg) => (
              <SelectItem
                key={pkg.value}
                value={pkg.value}
                className="font-mono text-xs"
              >
                {pkg.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </Field>
      <Field label={t("providers.form.geminiBaseUrl")}>
        <Input
          value={baseURL}
          onChange={(e) => onBaseUrlChange(e.target.value)}
          placeholder="https://api.example.com/v1"
          spellCheck={false}
        />
      </Field>
      <Field label={t("providers.form.apiKey")}>
        <Input
          type="password"
          value={apiKey}
          onChange={(e) => onApiKeyChange(e.target.value)}
          placeholder={t("providers.form.apiKeyPlaceholder")}
          spellCheck={false}
        />
      </Field>

      {/* headers 键值编辑器：每行 key + value，可增删。空 key 行留到填完再写盘。 */}
      <div className="flex flex-col gap-1.5">
        <Label className="text-muted-foreground text-xs">
          {t("providers.form.openCodeHeaders")}
        </Label>
        <p className="text-muted-foreground text-xs">
          {t("providers.form.openCodeHeadersHint")}
        </p>
        <div className="flex flex-col gap-2">
          {headerRows.map((row) => (
            <div key={row.id} className="flex items-center gap-2">
              <Input
                value={row.key}
                onChange={(e) => updateHeader(row.id, { key: e.target.value })}
                placeholder={t("providers.form.openCodeHeaderKey")}
                spellCheck={false}
                className="font-mono text-xs"
              />
              <Input
                value={row.value}
                onChange={(e) =>
                  updateHeader(row.id, { value: e.target.value })
                }
                placeholder={t("providers.form.openCodeHeaderValue")}
                spellCheck={false}
                className="font-mono text-xs"
              />
              <Button
                variant="ghost"
                size="icon"
                className="size-8 shrink-0 text-muted-foreground"
                onClick={() => removeHeader(row.id)}
                aria-label={t("common.delete")}
              >
                <Trash2 className="size-3.5" />
              </Button>
            </div>
          ))}
        </div>
        <Button
          variant="outline"
          size="sm"
          className="w-fit"
          onClick={addHeader}
        >
          <Plus className="size-3.5" />
          {t("providers.form.openCodeAddRow")}
        </Button>
      </div>

      {/* models 编辑器：每行 model_id + 显示名，可增删；fetch 按钮拉端点模型。 */}
      <div className="flex flex-col gap-1.5">
        <div className="flex items-center justify-between">
          <Label className="text-muted-foreground text-xs">
            {t("providers.form.openCodeModels")}
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
        <p className="text-muted-foreground text-xs">
          {t("providers.form.openCodeModelsHint")}
        </p>
        {fetchedModels.length > 0 ? (
          <Select
            onValueChange={(m) => typeof m === "string" && onPickModel(m)}
          >
            <SelectTrigger
              className="font-mono text-xs"
              aria-label={t("providers.form.fetchModels.geminiPlaceholder")}
            >
              <SelectValue
                placeholder={t("providers.form.fetchModels.geminiPlaceholder")}
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
        <div className="flex flex-col gap-2">
          {modelRows.map((row) => (
            <div key={row.id} className="flex items-center gap-2">
              <Input
                value={row.modelId}
                onChange={(e) =>
                  updateModel(row.id, { modelId: e.target.value })
                }
                placeholder={t("providers.form.openCodeModelId")}
                spellCheck={false}
                className="font-mono text-xs"
              />
              <Input
                value={row.name}
                onChange={(e) => updateModel(row.id, { name: e.target.value })}
                placeholder={t("providers.form.openCodeModelName")}
                spellCheck={false}
                className="text-xs"
              />
              <Button
                variant="ghost"
                size="icon"
                className="size-8 shrink-0 text-muted-foreground"
                onClick={() => removeModel(row.id)}
                aria-label={t("common.delete")}
              >
                <Trash2 className="size-3.5" />
              </Button>
            </div>
          ))}
        </div>
        <Button
          variant="outline"
          size="sm"
          className="w-fit"
          onClick={() => addModel()}
        >
          <Plus className="size-3.5" />
          {t("providers.form.openCodeAddRow")}
        </Button>
      </div>
    </>
  )
}

function Field({
  label,
  children,
}: {
  label: string
  children: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <Label className="text-muted-foreground text-xs">{label}</Label>
      {children}
    </div>
  )
}

/** headers map → 编辑器行（分配稳定 id）。 */
function toHeaderRows(map: Record<string, string>): HeaderRow[] {
  return Object.entries(map).map(([key, value]) => ({
    id: nextRowId(),
    key,
    value,
  }))
}

/** models map → 编辑器行（分配稳定 id；name 缺失归一为空串）。 */
function toModelRows(map: Record<string, { name?: string }>): ModelRow[] {
  return Object.entries(map).map(([modelId, m]) => ({
    id: nextRowId(),
    modelId,
    name: m.name ?? "",
  }))
}
