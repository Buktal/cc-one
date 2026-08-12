// OpenCode 应用内置预设清单（12 个）：随应用版本内置发布，不进同步、不进 DB。
// 预设 = 预填的 settingsConfig 快照（settingsConfig 是 JSON 文本，与
// Provider.settingsConfig 同构，形状为单条 provider entry：`{ npm, options:{
// baseURL, apiKey }, models }`——opencode.json 的 `provider.<key>` 子树内容，
// 与 claude/codex/grok 的「整 live 文件快照」不同）。选中预设后由表单层经
// derive.providerFromPreset 整份复制成「custom 分类」的新建草稿——预设常量
// 本身绝不被改动。
// 单一事实来源：数量、名称、分类、端点 / 模型映射都以本文件为准。
// 选取标准与 Claude / Codex / Gemini / Grok 侧一致：官方 + 国内大厂 + 热门聚合，
// 共 12 个（官方 3 + 国内大厂 6 + 热门聚合 3）。OpenCode 是 OpenAI 兼容生态，
// 国内大厂几乎都出 openai-compatible 端点，故国内收录比 Grok 丰富。
// 排除：OMO / OMO Slim（OpenCode 社区第三方互斥子机制，本应用不做）、
// 长尾 third_party（留给用户自定义）。端点取自 OpenCode CLI 实际使用的
// openai-compatible / 官方 SDK 端点（已对照 CC-Switch opencodeProviderPresets），
// 不是各家的 anthropic 兼容端点（那是 Claude 池用的）。

import type { ProviderPreset } from "@/features/providers/presets"

/** 一条 OpenCode provider entry（settingsConfig 形状）：`npm` + `options:{
 *  baseURL, apiKey:"" }` + 可选 `models`。`apiKey` 留空占位——OpenCode 是附加
 *  模式，预设只起「预填端点 / 包名 / 模型」作用，密钥由表单填、写盘走后端
 *  live_opencode 的单键 read-modify-write。空 models → 不带 models 键（聚合类
 *  供应商模型多且易变，交给「获取模型」按钮拉取）。 */
function openCodeEntry(
  npm: string,
  baseUrl: string,
  models: Record<string, { name?: string }> = {},
): string {
  const entry: Record<string, unknown> = {
    npm,
    options: { baseURL: baseUrl, apiKey: "" },
  }
  if (Object.keys(models).length > 0) entry.models = models
  return JSON.stringify(entry, null, 2)
}

export const OPENCODE_PROVIDER_PRESETS: ProviderPreset[] = [
  // ── 官方 3 ──
  {
    name: "OpenAI",
    category: "official",
    websiteUrl: "https://platform.openai.com",
    icon: "openai",
    iconColor: "#00A67E",
    notes: "OpenAI 官方端点（@ai-sdk/openai）：直连 api.openai.com。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai",
      "https://api.openai.com/v1",
      { "gpt-5.6-sol": { name: "GPT-5.6 Sol" } },
    ),
  },
  {
    name: "Anthropic",
    category: "official",
    websiteUrl: "https://www.anthropic.com",
    icon: "anthropic",
    iconColor: "#D4915D",
    notes: "Anthropic 官方端点（@ai-sdk/anthropic）：直连 api.anthropic.com。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/anthropic",
      "https://api.anthropic.com/v1",
      { "claude-opus-5": { name: "Claude Opus 5" } },
    ),
  },
  {
    name: "Google Gemini",
    category: "official",
    websiteUrl: "https://ai.google.dev",
    icon: "google",
    iconColor: "#1A73E8",
    notes: "Google Gemini 官方端点（@ai-sdk/google）：走 generativelanguage。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/google",
      "https://generativelanguage.googleapis.com/v1beta",
      { "gemini-3.6-flash": { name: "Gemini 3.6 Flash" } },
    ),
  },

  // ── 国内大厂 6 ──
  {
    name: "DeepSeek",
    category: "cn_official",
    websiteUrl: "https://platform.deepseek.com",
    icon: "deepseek",
    iconColor: "#1E88E5",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://api.deepseek.com/v1",
      {
        "deepseek-v4-pro": { name: "DeepSeek V4 Pro" },
        "deepseek-v4-flash": { name: "DeepSeek V4 Flash" },
      },
    ),
  },
  {
    name: "Kimi",
    category: "cn_official",
    websiteUrl: "https://platform.kimi.com",
    icon: "kimi",
    iconColor: "#6366F1",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://api.moonshot.cn/v1",
      { "kimi-k3": { name: "Kimi K3" } },
    ),
  },
  {
    name: "Zhipu GLM",
    category: "cn_official",
    websiteUrl: "https://open.bigmodel.cn",
    icon: "zhipu",
    iconColor: "#0F62FE",
    notes:
      "OpenCode 专用 coding 端点（/api/coding/paas/v4），非 anthropic 端点。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://open.bigmodel.cn/api/coding/paas/v4",
      { "glm-5.1": { name: "GLM-5.1" } },
    ),
  },
  {
    name: "阿里百炼",
    category: "cn_official",
    websiteUrl: "https://bailian.console.aliyun.com",
    icon: "bailian",
    iconColor: "#624AFF",
    notes: "阿里 DashScope OpenAI 兼容端点；模型用「获取模型」按钮拉取。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://dashscope.aliyuncs.com/compatible-mode/v1",
    ),
  },
  {
    name: "火山方舟",
    category: "cn_official",
    websiteUrl: "https://www.volcengine.com/product/ark",
    icon: "doubao",
    iconColor: "#3370FF",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://ark.cn-beijing.volces.com/api/v3",
      { "doubao-seed-2-1-pro-260628": { name: "Doubao Seed 2.1 Pro" } },
    ),
  },
  {
    name: "MiniMax",
    category: "cn_official",
    websiteUrl: "https://platform.minimaxi.com",
    icon: "minimax",
    iconColor: "#FF6B6B",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://api.minimax.io/v1",
      { "MiniMax-M2.7": { name: "MiniMax M2.7" } },
    ),
  },

  // ── 热门聚合 3 ──
  {
    name: "OpenRouter",
    category: "aggregator",
    websiteUrl: "https://openrouter.ai",
    icon: "openrouter",
    iconColor: "#6566F1",
    notes: "聚合众多模型；模型用「获取模型」按钮拉取。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://openrouter.ai/api/v1",
    ),
  },
  {
    name: "千象 Qiniu",
    category: "aggregator",
    websiteUrl: "https://www.qiniu.com",
    icon: "qiniu",
    iconColor: "#000000",
    settingsConfig: openCodeEntry(
      "@ai-sdk/openai-compatible",
      "https://api.qnaigc.com/v1",
    ),
  },
  {
    name: "AIHubMix",
    category: "aggregator",
    websiteUrl: "https://aihubmix.com",
    icon: "aihubmix",
    iconColor: "#1A1A1A",
    notes: "聚合众多模型；走 @ai-sdk/anthropic 包。",
    settingsConfig: openCodeEntry(
      "@ai-sdk/anthropic",
      "https://aihubmix.com/v1",
    ),
  },
]
