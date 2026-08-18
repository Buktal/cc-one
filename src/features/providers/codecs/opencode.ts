// OpenCode 应用的 settingsConfig codec：附加模式单条目 JSON 的文本级读写。
//
// OpenCode settingsConfig 形状与 claude（整份 settings.json 快照）/ codex /
// grok（整份 TOML）本质不同——它是 opencode.json 里 `provider.<key>` 子树的
// **内容**：`{ npm, options:{baseURL,apiKey,headers}, models:{<id>:{name}} }`。
// OpenCode 是附加模式（多供应商共存于 provider map，无唯一活跃），写盘走后端
// 独立的单键 read-modify-write（live_opencode），前端只编辑这一个子树。`models`
// 在 entry 顶层（与 npm/options 平级），是 `model_id → { name, ... }` 的 map——
// 与 fetch_models 拉回的字符串数组（下拉用）不是一回事。

import { parseJsonObjectLenient } from "@/lib/json"

/** OpenCode entry 的一条模型：`models.<id> = { name?, ... }`。UI 第一版只编辑
 *  name，子条目里的其余字段（contextWindow 等）不暴露。 */
export type OpenCodeModelEntry = { name?: string }

/** OpenCode settingsConfig 的结构化视图（供表单读取）：npm 包名 + options 三件
 *  套（baseURL/apiKey/headers）+ models map。写入经 withOpenCode* 系列，保留
 *  entry 顶层与 options 内的非受控键（后端「只动目标键、其它保留」不变量在前端
 *  的镜像）。 */
export type OpenCodeConfig = {
  npm: string
  baseURL: string
  apiKey: string
  headers: Record<string, string>
  models: Record<string, OpenCodeModelEntry>
}

/** 解析 OpenCode settingsConfig 文本为 entry 原始对象（容错）：空 / 垃圾 / 非对象
 *  → `{}`——表单遇到手改坏的快照也不崩，写回时按归一结果继续。与
 *  `parseCodexConfig` / `parseGrokConfig` 同一「先 parse 再说」形态。 */
export function parseOpenCodeEntry(text: string): Record<string, unknown> {
  return parseJsonObjectLenient(text) ?? {}
}

/** 读 entry 的 `options` 子对象（容错为 `{}`）：非对象 options（手写垃圾）按空
 *  对象，与后端合并语义一致（非对象 options 不参与键级合并）。导出供必填检查
 *  （missing.ts）读 `options.baseURL`。 */
export function openCodeOptionsOf(
  entry: Record<string, unknown>,
): Record<string, unknown> {
  const options = entry.options
  if (
    options !== null &&
    typeof options === "object" &&
    !Array.isArray(options)
  ) {
    return options as Record<string, unknown>
  }
  return {}
}

/** 把一个 unknown 归一为 `Record<string,string>`（只保留字符串值，其余丢弃）——
 *  headers 键值对的读取契约（与 codex auth 的字符串过滤同一手法）。 */
function stringRecord(value: unknown): Record<string, string> {
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    )
  }
  return {}
}

/** 把 entry 的 `models` 子对象归一为 `model_id → { name? }`：非对象 / 缺失 → `{}`；
 *  每个子条目非对象 → `{}`，对象但 name 非字符串 → `{}`（丢 name）。 */
function openCodeModelsOf(
  entry: Record<string, unknown>,
): Record<string, OpenCodeModelEntry> {
  const raw = entry.models
  if (raw === null || typeof raw !== "object" || Array.isArray(raw)) return {}
  return Object.fromEntries(
    Object.entries(raw as Record<string, unknown>).map(([id, v]) => {
      if (
        v !== null &&
        typeof v === "object" &&
        !Array.isArray(v) &&
        typeof (v as Record<string, unknown>).name === "string"
      ) {
        return [id, { name: (v as Record<string, { name?: string }>).name }]
      }
      return [id, {}]
    }),
  )
}

/** 解析 OpenCode settingsConfig 文本为结构化视图（npm/baseURL/apiKey/headers/
 *  models）。宽容：空 / 垃圾 / 非对象 → 各字段归零；非对象 options / models /
 *  headers 按空——表单遇手改坏的快照也不崩。 */
export function parseOpenCodeConfig(text: string): OpenCodeConfig {
  const entry = parseOpenCodeEntry(text)
  const options = openCodeOptionsOf(entry)
  return {
    npm: typeof entry.npm === "string" ? entry.npm : "",
    baseURL: typeof options.baseURL === "string" ? options.baseURL : "",
    apiKey: typeof options.apiKey === "string" ? options.apiKey : "",
    headers: stringRecord(options.headers),
    models: openCodeModelsOf(entry),
  }
}

/** 读 OpenCode entry 的 npm 包名（`@ai-sdk/openai-compatible` 等 AI SDK 包）。 */
export function openCodeNpm(text: string): string {
  return parseOpenCodeConfig(text).npm
}

/** 读 OpenCode entry 的 `options.baseURL`（端点）。 */
export function openCodeBaseUrl(text: string): string {
  return parseOpenCodeConfig(text).baseURL
}

/** 读 OpenCode entry 的 `options.apiKey`。 */
export function openCodeApiKey(text: string): string {
  return parseOpenCodeConfig(text).apiKey
}

/** 读 OpenCode entry 的 `options.headers`（键值对，仅字符串值）。 */
export function openCodeHeaders(text: string): Record<string, string> {
  return parseOpenCodeConfig(text).headers
}

/** 读 OpenCode entry 的 `models` map（`model_id → { name? }`）。 */
export function openCodeModels(
  text: string,
): Record<string, OpenCodeModelEntry> {
  return parseOpenCodeConfig(text).models
}

/** 在 entry 文本上改写 options（保留 entry 顶层其它键 + options 内其它键）的共享
 *  引擎——所有 options.* 写入器经此，与 claude 的 `withEnvInText` 同一「先 parse
 *  再 spread」形态，避免各写入器各自重复而漂移。 */
function withOpenCodeOptions(
  text: string,
  write: (options: Record<string, unknown>) => void,
): string {
  const entry = parseOpenCodeEntry(text)
  const next = { ...openCodeOptionsOf(entry) }
  write(next)
  return JSON.stringify({ ...entry, options: next }, null, 2)
}

/** 写 npm 包名：保留 options / models / 顶层其它键（如 name）不动。空串 → 删除
 *  npm 键（回归无包名）。 */
export function withOpenCodeNpm(text: string, npm: string): string {
  const entry = parseOpenCodeEntry(text)
  const next = { ...entry }
  if (npm) next.npm = npm
  else delete next.npm
  return JSON.stringify(next, null, 2)
}

/** 写 `options.baseURL`：保留 options 其它键 + entry 顶层其它键。空串 → 删键。 */
export function withOpenCodeBaseUrl(text: string, baseURL: string): string {
  return withOpenCodeOptions(text, (options) => {
    if (baseURL) options.baseURL = baseURL
    else delete options.baseURL
  })
}

/** 写 `options.apiKey`：保留 options 其它键 + entry 顶层其它键。空串 → 删键
 *  （回归无 key 版，OpenCode CLI 会回落到 auth.json 登录态）。 */
export function withOpenCodeApiKey(text: string, apiKey: string): string {
  return withOpenCodeOptions(text, (options) => {
    if (apiKey) options.apiKey = apiKey
    else delete options.apiKey
  })
}

/** 写 `options.headers`（整块替换键值对）：空对象 → 删 `options.headers` 键，
 *  非空 → 写入。保留 options 其它键 + entry 顶层其它键。 */
export function withOpenCodeHeaders(
  text: string,
  headers: Record<string, string>,
): string {
  return withOpenCodeOptions(text, (options) => {
    if (Object.keys(headers).length > 0) options.headers = headers
    else delete options.headers
  })
}

/** 写 `models`（顶层 `model_id → {name}` map，整块替换）：空对象 → 删 `models`
 *  键；非空 → 写入（空 name 的条目写成 `{}`，OpenCode CLI 容忍）。保留 entry
 *  顶层其它键 + options 不动。空白 model_id 被丢弃。 */
export function withOpenCodeModels(
  text: string,
  models: Record<string, OpenCodeModelEntry>,
): string {
  const entry = parseOpenCodeEntry(text)
  const next = { ...entry }
  const out: Record<string, { name?: string }> = {}
  for (const [id, m] of Object.entries(models)) {
    const trimmed = id.trim()
    if (!trimmed) continue
    out[trimmed] = m.name ? { name: m.name } : {}
  }
  if (Object.keys(out).length > 0) next.models = out
  else delete next.models
  return JSON.stringify(next, null, 2)
}
