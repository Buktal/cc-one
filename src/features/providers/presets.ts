// biome-ignore-all lint/suspicious/noTemplateCurlyInString: 模板变量占位符（如
// `${AWS_REGION}`）是字面量文本，须保留到模板变量替换步骤，不是 JS 模板插值。
// 内置供应商预设清单（18 个）：随应用版本内置发布，不进同步、不进 DB。
// 预设 = 预填的 settings.json 快照（settingsConfig 是 JSON 文本，与
// Provider.settingsConfig 同构）。选中预设后由表单层经 derive.providerFromPreset
// 整份复制成「custom 分类」的新建草稿——预设常量本身绝不被改动。
// 单一事实来源：数量、名称、分类、端点 / 模型映射都以本文件为准。
// 排除项（OAuth 类 GitHub Copilot/Codex/xAI、gemini_native、openai_chat 格式）
// 一个都不进清单——它们需要本地代理或 OAuth，本功能不做。

import { CODEX_PROVIDER_PRESETS } from "@/features/providers/codex-presets"
import { GEMINI_PROVIDER_PRESETS } from "@/features/providers/gemini-presets"
import type { App, ProviderCategory } from "@/types/generated/bindings"

/** 一个内置预设：字段对齐 Provider，但没有 id / sortIndex / updatedAt——
 *  预设不是持久化实体，落表单时才被复制成供应商草稿。 */
export type ProviderPreset = {
  name: string
  category: ProviderCategory
  websiteUrl: string
  icon: string
  iconColor: string
  notes?: string
  /** 预填的 settings.json 快照（JSON 文本），含 env 块：baseURL、认证字段占位、模型映射等。 */
  settingsConfig: string
  /** 模型列表端点覆写：默认端点拼不出正确的 OpenAI 兼容候选时（如
   *  `/api/compatible` 不在候选构造的剥离清单里）精确指路。表单「获取模型
   *  列表」在端点等于本预设默认 ANTHROPIC_BASE_URL 时优先带上它。 */
  modelsUrl?: string
}

/** 把 env 块（外加可选顶层字段）序列化成 settings.json 快照文本。 */
function snapshot(
  env: Record<string, string>,
  extra?: Record<string, unknown>,
): string {
  return JSON.stringify({ ...extra, env }, null, 2)
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  // ── 官方 / 云 3 ──
  {
    name: "Claude Official",
    category: "official",
    websiteUrl: "https://www.anthropic.com/claude-code",
    icon: "anthropic",
    iconColor: "#D4915D",
    notes: "Anthropic 官方端点：留空即走默认，无需填写端点或模型。",
    settingsConfig: snapshot({}),
  },
  {
    name: "AWS Bedrock (AKSK)",
    category: "cloud_provider",
    websiteUrl: "https://aws.amazon.com/bedrock/",
    icon: "aws",
    iconColor: "#FF9900",
    notes:
      "走 AWS 访问密钥认证；AWS_REGION / AK / SK 为模板变量，保存前需填写。",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
      AWS_ACCESS_KEY_ID: "${AWS_ACCESS_KEY_ID}",
      AWS_SECRET_ACCESS_KEY: "${AWS_SECRET_ACCESS_KEY}",
      AWS_REGION: "${AWS_REGION}",
      ANTHROPIC_MODEL: "global.anthropic.claude-opus-5",
      ANTHROPIC_DEFAULT_HAIKU_MODEL:
        "global.anthropic.claude-haiku-4-5-20251001-v1:0",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "global.anthropic.claude-sonnet-5",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "global.anthropic.claude-opus-5",
      CLAUDE_CODE_USE_BEDROCK: "1",
    }),
  },
  {
    name: "AWS Bedrock (API Key)",
    category: "cloud_provider",
    websiteUrl: "https://aws.amazon.com/bedrock/",
    icon: "aws",
    iconColor: "#FF9900",
    notes: "以 API Key 走 Bedrock；AWS_REGION 为模板变量，保存前需填写。",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://bedrock-runtime.${AWS_REGION}.amazonaws.com",
      AWS_REGION: "${AWS_REGION}",
      ANTHROPIC_MODEL: "global.anthropic.claude-opus-5",
      ANTHROPIC_DEFAULT_HAIKU_MODEL:
        "global.anthropic.claude-haiku-4-5-20251001-v1:0",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "global.anthropic.claude-sonnet-5",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "global.anthropic.claude-opus-5",
      CLAUDE_CODE_USE_BEDROCK: "1",
    }),
  },

  // ── 国内大厂 11 ──
  {
    name: "Kimi",
    category: "cn_official",
    websiteUrl: "https://platform.kimi.com",
    icon: "kimi",
    iconColor: "#6366F1",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.moonshot.cn/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "kimi-k2.7-code",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "kimi-k2.7-code",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "kimi-k2.7-code",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "kimi-k2.7-code",
    }),
  },
  {
    name: "Kimi For Coding",
    category: "cn_official",
    websiteUrl: "https://www.kimi.com/code/",
    icon: "kimi",
    iconColor: "#6366F1",
    notes: "双键钉 256K 上下文压缩窗口；模型走路由端点别名 kimi-for-coding。",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.kimi.com/coding/",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "kimi-for-coding",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "kimi-for-coding",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "kimi-for-coding",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "kimi-for-coding",
      CLAUDE_CODE_MAX_CONTEXT_TOKENS: "262144",
      CLAUDE_CODE_AUTO_COMPACT_WINDOW: "262144",
    }),
  },
  {
    name: "DeepSeek",
    category: "cn_official",
    websiteUrl: "https://platform.deepseek.com",
    icon: "deepseek",
    iconColor: "#1E88E5",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.deepseek.com/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "deepseek-v4-pro",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "deepseek-v4-flash",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "deepseek-v4-pro",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "deepseek-v4-pro",
    }),
  },
  {
    name: "Zhipu GLM",
    category: "cn_official",
    websiteUrl: "https://open.bigmodel.cn",
    icon: "zhipu",
    iconColor: "#0F62FE",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://open.bigmodel.cn/api/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "glm-5.1",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "glm-5.1",
    }),
  },
  {
    name: "火山 Agentplan",
    category: "cn_official",
    websiteUrl: "https://www.volcengine.com/product/ark",
    icon: "huoshan",
    iconColor: "#3370FF",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://ark.cn-beijing.volces.com/api/coding",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "ark-code-latest",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "ark-code-latest",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "ark-code-latest",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "ark-code-latest",
    }),
  },
  {
    name: "DouBaoSeed",
    category: "cn_official",
    websiteUrl: "https://console.volcengine.com/ark",
    icon: "doubao",
    iconColor: "#3370FF",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://ark.cn-beijing.volces.com/api/compatible",
      ANTHROPIC_AUTH_TOKEN: "",
      API_TIMEOUT_MS: "3000000",
      ANTHROPIC_MODEL: "doubao-seed-2-1-pro-260628",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "doubao-seed-2-1-pro-260628",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "doubao-seed-2-1-pro-260628",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "doubao-seed-2-1-pro-260628",
    }),
    // /api/compatible 不在候选构造的 9 种剥离后缀里，拼不出正确候选，
    // 精确指路到火山 OpenAI 兼容的 /api/v3/models。
    modelsUrl: "https://ark.cn-beijing.volces.com/api/v3/models",
  },
  {
    name: "百度千帆",
    category: "cn_official",
    websiteUrl: "https://cloud.baidu.com/product/qianfan",
    icon: "baidu",
    iconColor: "#2932E1",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://qianfan.baidubce.com/anthropic/coding",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "qianfan-code-latest",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "qianfan-code-latest",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "qianfan-code-latest",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "qianfan-code-latest",
    }),
  },
  {
    name: "阿里百炼 For Coding",
    category: "cn_official",
    websiteUrl: "https://bailian.console.aliyun.com",
    icon: "bailian",
    iconColor: "#624AFF",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL:
        "https://coding.dashscope.aliyuncs.com/apps/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
    }),
  },
  {
    name: "StepFun",
    category: "cn_official",
    websiteUrl: "https://platform.stepfun.com/step-plan",
    icon: "stepfun",
    iconColor: "#16D6D2",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.stepfun.com/step_plan",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "step-3.5-flash-2603",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "step-3.5-flash-2603",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "step-3.5-flash-2603",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "step-3.5-flash-2603",
    }),
  },
  {
    name: "MiniMax",
    category: "cn_official",
    websiteUrl: "https://platform.minimaxi.com",
    icon: "minimax",
    iconColor: "#FF6B6B",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.minimaxi.com/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
      API_TIMEOUT_MS: "3000000",
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
      ANTHROPIC_MODEL: "MiniMax-M2.7",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "MiniMax-M2.7",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "MiniMax-M2.7",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "MiniMax-M2.7",
    }),
  },
  {
    name: "小米 MiMo",
    category: "cn_official",
    websiteUrl: "https://platform.xiaomimimo.com",
    icon: "xiaomimimo",
    iconColor: "#000000",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.xiaomimimo.com/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "mimo-v2.5-pro",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "mimo-v2.5-pro",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "mimo-v2.5-pro",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "mimo-v2.5-pro",
    }),
  },

  // ── 热门聚合 4 ──
  {
    name: "SiliconFlow",
    category: "aggregator",
    websiteUrl: "https://siliconflow.cn",
    icon: "siliconflow",
    iconColor: "#6E29F6",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.siliconflow.cn",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "Pro/MiniMaxAI/MiniMax-M2.7",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "Pro/MiniMaxAI/MiniMax-M2.7",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "Pro/MiniMaxAI/MiniMax-M2.7",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "Pro/MiniMaxAI/MiniMax-M2.7",
    }),
  },
  {
    name: "OpenRouter",
    category: "aggregator",
    websiteUrl: "https://openrouter.ai",
    icon: "openrouter",
    iconColor: "#6566F1",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://openrouter.ai/api",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "anthropic/claude-sonnet-5",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "anthropic/claude-haiku-4.5",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "anthropic/claude-sonnet-5",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "anthropic/claude-opus-5",
    }),
  },
  {
    name: "ModelScope",
    category: "aggregator",
    websiteUrl: "https://modelscope.cn",
    icon: "modelscope",
    iconColor: "#624AFF",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api-inference.modelscope.cn",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "ZhipuAI/GLM-5.1",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "ZhipuAI/GLM-5.1",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "ZhipuAI/GLM-5.1",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "ZhipuAI/GLM-5.1",
    }),
  },
  {
    name: "Novita AI",
    category: "aggregator",
    websiteUrl: "https://novita.ai",
    icon: "novita",
    iconColor: "#000000",
    settingsConfig: snapshot({
      ANTHROPIC_BASE_URL: "https://api.novita.ai/anthropic",
      ANTHROPIC_AUTH_TOKEN: "",
      ANTHROPIC_MODEL: "zai-org/glm-5.1",
      ANTHROPIC_DEFAULT_HAIKU_MODEL: "zai-org/glm-5.1",
      ANTHROPIC_DEFAULT_SONNET_MODEL: "zai-org/glm-5.1",
      ANTHROPIC_DEFAULT_OPUS_MODEL: "zai-org/glm-5.1",
    }),
  },
]

/** 按应用返回对应的内置预设清单：claude → 18 个、codex → 17 个、gemini →
 *  6 个。单一事实来源的入口——调用方一律走这里，不要直接读三个常量。 */
export function presetsForApp(app: App): ProviderPreset[] {
  switch (app) {
    case "claude":
      return PROVIDER_PRESETS
    case "codex":
      return CODEX_PROVIDER_PRESETS
    case "gemini":
      return GEMINI_PROVIDER_PRESETS
  }
}
