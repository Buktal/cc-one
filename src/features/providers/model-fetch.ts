// 模型列表获取的前端纯逻辑：modelsUrl 覆写推导 + 失败错误串分桶。
// 网络请求本身由后端命令 fetch_models_cmd 发出（WebView fetch 撞 CORS，
// 必须走后端）；本文件只有两个可测的纯函数，不碰任何 tauri 句柄。
//
// 错误串分桶契约：后端错误串带稳定前缀标签（AUTH_FAILED / ENDPOINT_CLOSED
// / TIMEOUT / BAD_FORMAT / NETWORK），这里按前缀映射到提示桶；无标签 /
// 未知标签一律落 network 兜底，detail 保留原串不吞信息。标签清单与
// src-tauri/src/provider/model_fetch.rs 的模块文档一一对应。

// 直连 codecs/claude（不经 derive 聚合层）：model-fetch 是被 app-profiles
// 引用的底层域，保持对聚合层零依赖，避免 derive ↔ app-profiles 循环。
import { configEndpoint } from "@/features/providers/codecs/claude"
import type { ProviderPreset } from "@/features/providers/presets"

import type { App } from "@/types/generated/bindings"

/** 去首尾空白 + 尾部斜杠，端点比较用（候选构造也这么处理 baseURL）。 */
function normalizeEndpoint(endpoint: string): string {
  return endpoint.trim().replace(/\/+$/, "")
}

/** modelsUrl 覆写推导：端点等于某预设的默认 ANTHROPIC_BASE_URL（归一化
 *  后），且该预设声明了 modelsUrl 时返回它。这些预设的端点拼不出正确的
 *  OpenAI 兼容候选（如火山 `/api/compatible` 不在剥离清单里），必须精确
 *  指路。不匹配任何预设、或匹配的预设没有 modelsUrl → null。 */
export function presetModelsUrl(
  endpoint: string,
  presets: ProviderPreset[],
): string | null {
  const trimmed = normalizeEndpoint(endpoint)
  if (!trimmed) return null
  for (const preset of presets) {
    if (!preset.modelsUrl) continue
    if (normalizeEndpoint(configEndpoint(preset.settingsConfig)) === trimmed) {
      return preset.modelsUrl
    }
  }
  return null
}

/** 一次 fetch_models 调用的完整参数（app + 端点 + 认证 + modelsUrl 覆写）。
 *  per-app 提取见 app-profiles 的 modelFetch 行。 */
export interface FetchModelsArgs {
  app: App
  baseUrl: string
  apiKey: string
  modelsUrl: string | null
}

/** 拉模型参数的提取结果。判别联合：`ok: true` 时 args 必存在、`ok: false`
 *  时 missing 给出缺的部分（endpoint / key，调用方提示对应文案）——互斥
 *  不变量在类型里，不用 `!`。 */
export type FetchArgsResult =
  | { ok: true; args: FetchModelsArgs }
  | { ok: false; missing: "endpoint" | "key" }

/** 模型获取失败的分桶类型——决定 toast 标题。 */
export type ModelsFetchErrorKind =
  | "auth"
  | "endpoint"
  | "timeout"
  | "format"
  | "network"

/** 分桶结果：kind 决定 toast 标题，detail（标签剥离后的原文）作描述。 */
export interface BucketedFetchError {
  kind: ModelsFetchErrorKind
  detail: string
}

/** 后端错误串的前缀标签 → 提示桶。 */
const FETCH_ERROR_TAGS: Record<string, ModelsFetchErrorKind> = {
  AUTH_FAILED: "auth",
  ENDPOINT_CLOSED: "endpoint",
  TIMEOUT: "timeout",
  BAD_FORMAT: "format",
  NETWORK: "network",
}

/** 把后端错误串分桶：`TAG: detail` 按标签映射，无标签 / 未知标签一律落
 *  network 兜底（detail 保留原串）。 */
export function bucketFetchModelsError(message: string): BucketedFetchError {
  const colon = message.indexOf(": ")
  if (colon > 0) {
    const kind = FETCH_ERROR_TAGS[message.slice(0, colon)]
    if (kind) return { kind, detail: message.slice(colon + 2) }
  }
  return { kind: "network", detail: message }
}
