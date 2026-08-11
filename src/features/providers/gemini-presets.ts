// Gemini 应用内置预设清单（6 个）：随应用版本内置发布，不进同步、不进 DB。
// 预设 = 预填的 settingsConfig 快照（settingsConfig 是 JSON 文本，与
// Provider.settingsConfig 同构，形状为 `{"env": {...}, "config"?: {...}}`）。
// env 整块写 ~/.gemini/.env，config 合并进 settings.json 的受控字段。
// 选中预设后由表单层经 derive.providerFromPreset 整份复制成「custom 分类」的
// 新建草稿——预设常量本身绝不被改动。
// 单一事实来源：数量、名称、分类、端点 / 模型映射都以本文件为准。
// 配置值与 env 形状照搬 CC-Switch 的 geminiProviderPresets.ts，仅做应用维度
// 的精选（官方 2 + 热门聚合 4 = 6）。

import type { ProviderPreset } from "@/features/providers/presets"

/** 把 Gemini 写盘快照序列化成 settingsConfig JSON 文本：env 整块写 .env，
 *  可选 config 合并进 settings.json 的受控字段。空 env 且无 config → `"{}"`，
 *  与 Rust parse_gemini_settings 对空快照的宽容契约一致。 */
function geminiSnapshot(
  env: Record<string, string>,
  config?: Record<string, unknown>,
): string {
  if (!Object.keys(env).length && !config) return "{}"
  return JSON.stringify(config ? { env, config } : { env }, null, 2)
}

export const GEMINI_PROVIDER_PRESETS: ProviderPreset[] = [
  // ── 官方 2 ──
  {
    name: "Google Gemini",
    category: "official",
    websiteUrl: "https://ai.google.dev/",
    icon: "gemini",
    iconColor: "#4285F4",
    notes: "Google 官方 Gemini：走 OAuth 登录态，无需 API Key。",
    settingsConfig: geminiSnapshot({}),
  },
  {
    name: "Google Gemini (API Key)",
    category: "official",
    websiteUrl: "https://ai.google.dev/",
    icon: "gemini",
    iconColor: "#4285F4",
    notes:
      "Google 官方 Gemini（API Key 版）：直连 generativelanguage.googleapis.com。",
    settingsConfig: geminiSnapshot({
      GEMINI_API_KEY: "",
      GOOGLE_GEMINI_BASE_URL: "https://generativelanguage.googleapis.com",
      GEMINI_MODEL: "gemini-3.6-pro",
    }),
  },

  // ── 热门聚合 4 ──
  {
    name: "OpenRouter",
    category: "aggregator",
    websiteUrl: "https://openrouter.ai",
    icon: "openrouter",
    iconColor: "#6566F1",
    settingsConfig: geminiSnapshot({
      GOOGLE_GEMINI_BASE_URL: "https://openrouter.ai/api",
      GEMINI_MODEL: "gemini-3.6-flash",
    }),
  },
  {
    name: "TheRouter",
    category: "aggregator",
    websiteUrl: "https://therouter.ai",
    icon: "therouter",
    iconColor: "#000000",
    settingsConfig: geminiSnapshot({
      GOOGLE_GEMINI_BASE_URL: "https://api.therouter.ai",
      GEMINI_MODEL: "gemini-3.6-flash",
    }),
  },
  {
    name: "千象 Qiniu",
    category: "aggregator",
    websiteUrl: "https://www.qiniu.com",
    icon: "qiniu",
    iconColor: "#000000",
    settingsConfig: geminiSnapshot({
      GOOGLE_GEMINI_BASE_URL: "https://api.qnaigc.com/bypass/vertex",
      GEMINI_MODEL: "gemini-3.6-flash",
    }),
  },
  {
    name: "E-FlowCode",
    category: "aggregator",
    websiteUrl: "https://e-flowcode.cc",
    icon: "eflowcode",
    iconColor: "#000000",
    settingsConfig: geminiSnapshot(
      {
        GOOGLE_GEMINI_BASE_URL: "https://e-flowcode.cc",
        GEMINI_API_KEY: "",
        GEMINI_MODEL: "gemini-3.6-flash",
      },
      {
        general: {
          previewFeatures: true,
          sessionRetention: {
            enabled: true,
            maxAge: "30d",
            warningAcknowledged: true,
          },
        },
      },
    ),
  },
]
