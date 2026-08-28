// Grok 应用内置预设清单（6 个）：随应用版本内置发布，不进同步、不进 DB。
// 预设 = 预填的 settingsConfig 快照（settingsConfig 是 JSON 文本，与
// Provider.settingsConfig 同构，形状为 `{"config": "<TOML>"}`）。config 即
// ~/.grok/config.toml 的受控片段。选中预设后由表单层经 derive.providerFromPreset
// 整份复制成「custom 分类」的新建草稿——预设常量本身绝不被改动。
// 单一事实来源：数量、名称、分类、端点 / 模型映射都以本文件为准。
// 选取标准与 Claude / Codex / Gemini 侧一致：官方 + 国内大厂 + 热门聚合。
// Grok 池**无国内大厂**（DeepSeek/Kimi/GLM 等未出 grok 兼容端点），故只收
// 官方 2 + 已验证热门聚合 4，共 6 个——不为凑数降门槛。需本地代理注入 token
// 的托管 OAuth 不收（本应用无本地代理）。
//
// 配置格式说明：Grok 的 ~/.grok/config.toml 是「命名 profile」式 TOML——
// `[models]` 表的 `default = "<profile>"` 指向当前激活 profile，`[model.<name>]`
// 是各供应商块（含 `api_backend` / `context_window` 等 grok 专属字段，与 codex
// 的 `model_providers` 形态不同）。cc one 固定写 canonical profile `cc-one`，
// 切换时整块替换该 profile + 设 `models.default`（写盘在后端 live_grok 完成）；
// 故预设只携带 `[model.cc-one]` 块，`models.default` 由写盘层补。

import type { ProviderPreset } from "@/features/providers/presets"

/** Grok profile 块默认值：`api_backend` 用 `responses`（Grok CLI 的 wire
 *  协议后端），`context_window` 用 500_000（典型上下文窗口大小）。 */
const DEFAULT_API_BACKEND = "responses"
const DEFAULT_CONTEXT_WINDOW = 500_000

/** 第三方供应商的 `[model.cc-one]` profile 块 TOML：model / base_url / api_key
 *  / api_backend / context_window / name 六字段。这些就是写盘的受控内容（整块
 *  替换 ~/.grok/config.toml 的 cc-one profile + 设 models.default）；用户手动
 *  的其它 profile / mcp_servers 等非受控字段写盘时原样保留。
 *  `api_key = ""` 是占位（API Key 版，表单填值）。 */
function grokProfile(
  name: string,
  baseUrl: string,
  model = "grok-4.5",
): string {
  const s = (value: string) => JSON.stringify(value)
  return `[model.cc-one]
model = ${s(model)}
base_url = ${s(baseUrl)}
api_key = ""
api_backend = ${s(DEFAULT_API_BACKEND)}
context_window = ${DEFAULT_CONTEXT_WINDOW}
name = ${s(name)}`
}

/** 把 Grok 写盘 TOML 片段序列化成 settingsConfig JSON 文本：`{"config": "<toml>"}`。
 *  空 toml → `"{}"`（登录态版：无自定义 profile，Grok CLI 回落自带 xAI OAuth
 *  订阅登录）。 */
function grokSnapshot(toml: string): string {
  if (!toml) return "{}"
  return JSON.stringify({ config: toml }, null, 2)
}

export const GROK_PROVIDER_PRESETS: ProviderPreset[] = [
  // ── 官方 2 ──
  {
    name: "Grok Official",
    category: "official",
    websiteUrl: "https://x.ai/grok",
    icon: "grok",
    iconColor: "#1f1f1f",
    settingsConfig: grokSnapshot(""),
  },
  {
    name: "xAI (Grok)",
    category: "official",
    websiteUrl: "https://x.ai/api",
    icon: "grok",
    iconColor: "#1f1f1f",
    settingsConfig: grokSnapshot(grokProfile("xAI", "https://api.x.ai/v1")),
  },

  // ── 热门聚合 4 ──
  {
    name: "OpenRouter",
    category: "aggregator",
    websiteUrl: "https://openrouter.ai",
    icon: "openrouter",
    iconColor: "#6566F1",
    settingsConfig: grokSnapshot(
      grokProfile(
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        "x-ai/grok-4.5",
      ),
    ),
  },
  {
    name: "TheRouter",
    category: "aggregator",
    websiteUrl: "https://therouter.ai",
    icon: "therouter",
    iconColor: "#000000",
    settingsConfig: grokSnapshot(
      grokProfile("TheRouter", "https://api.therouter.ai/v1", "x-ai/grok-4.5"),
    ),
  },
  {
    name: "千象 Qiniu",
    category: "aggregator",
    websiteUrl: "https://www.qiniu.com",
    icon: "qiniu",
    iconColor: "#000000",
    settingsConfig: grokSnapshot(
      grokProfile("千象", "https://api.qnaigc.com/bypass/openai/v1"),
    ),
  },
  {
    name: "E-FlowCode",
    category: "aggregator",
    websiteUrl: "https://e-flowcode.cc",
    icon: "eflowcode",
    iconColor: "#000000",
    settingsConfig: grokSnapshot(
      grokProfile("E-FlowCode", "https://e-flowcode.cc/v1"),
    ),
  },
]
