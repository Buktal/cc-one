// Codex 应用内置预设清单（17 个）：随应用版本内置发布，不进同步、不进 DB。
// 预设 = 预填的 settingsConfig 快照（settingsConfig 是 JSON 文本，与
// Provider.settingsConfig 同构，形状为 `{"auth": {...}, "config": "<TOML>"}`）。
// 选中预设后由表单层经 derive.providerFromPreset 整份复制成「custom 分类」的
// 新建草稿——预设常量本身绝不被改动。
// 单一事实来源：数量、名称、分类、端点 / 模型映射都以本文件为准。
// 选取标准与 Claude 侧一致：官方 + 国内大厂 + 热门聚合，共 17 个（官方 2
// + 国内大厂 11 + 热门聚合 4）。需要本地代理注入 token 的托管 OAuth
// （xAI 等）不收——本应用无本地代理。

import type { ProviderPreset } from "@/features/providers/presets"

/** 第三方供应商的 config.toml 模板：model_provider / model /
 *  model_reasoning_effort / disable_response_storage 顶层四件套 +
 *  [model_providers.custom] 表五字段。这些键都是切换写盘的受控键（整块替换
 *  进 ~/.codex/config.toml）；用户手动的 mcp_servers / web_search 等非受控
 *  字段写盘时原样保留。 */
function thirdPartyConfig(
  name: string,
  baseUrl: string,
  model = "gpt-5.6-sol",
): string {
  const tomlString = (value: string) => JSON.stringify(value)
  return `model_provider = "custom"
model = ${tomlString(model)}
model_reasoning_effort = "high"
disable_response_storage = true

[model_providers.custom]
name = ${tomlString(name)}
base_url = ${tomlString(baseUrl)}
wire_api = "responses"
requires_openai_auth = true`
}

/** 把 Codex 写盘快照（auth + config TOML）序列化成 settingsConfig JSON 文本。
 *  登录态版（无 key 且无 config）→ `"{}"`：空快照即「无受控内容」，写盘时
 *  不碰 auth.json、不写 config.toml，保留既有 ChatGPT 登录态；其余情形产出
 *  `{"auth": {...}, "config": "<toml>"}`。
 *  `withKey=true` 时 auth 带 `OPENAI_API_KEY: ""` 占位（API Key 版，表单填值）；
 *  `withKey=false` 时 auth 为 `{}`（登录态版，不写 auth.json）。 */
function codexSnapshot(toml: string, withKey = false): string {
  if (!withKey && !toml) return "{}"
  const auth = withKey ? { OPENAI_API_KEY: "" } : {}
  return JSON.stringify({ auth, config: toml }, null, 2)
}

export const CODEX_PROVIDER_PRESETS: ProviderPreset[] = [
  // ── 官方 2 ──
  {
    // 同为 OpenAI 官方端点的两个预设靠括注认证方式区分（对齐 Claude 侧
    // "AWS Bedrock (AKSK)/(API Key)" 的既有命名式）：ChatGPT 登录态版 vs
    // API Key 直连版，列表里看名称括注即可分辨。
    name: "OpenAI (ChatGPT 登录)",
    category: "official",
    websiteUrl: "https://chatgpt.com/codex",
    icon: "openai",
    iconColor: "#00A67E",
    settingsConfig: codexSnapshot(""),
  },
  {
    name: "OpenAI (API Key)",
    category: "official",
    websiteUrl: "https://platform.openai.com",
    icon: "openai",
    iconColor: "#00A67E",
    settingsConfig: codexSnapshot(
      thirdPartyConfig("openai", "https://api.openai.com/v1", "gpt-5.6-sol"),
      true,
    ),
  },

  // ── 国内大厂 11 ──
  {
    name: "Kimi",
    category: "cn_official",
    websiteUrl: "https://platform.kimi.com",
    icon: "kimi",
    iconColor: "#6366F1",
    settingsConfig: codexSnapshot(
      thirdPartyConfig("kimi", "https://api.moonshot.cn/v1", "kimi-k2.7-code"),
      true,
    ),
  },
  {
    name: "Kimi For Coding",
    category: "cn_official",
    websiteUrl: "https://www.kimi.com/code/",
    icon: "kimi",
    iconColor: "#6366F1",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "kimi_coding",
        "https://api.kimi.com/coding/v1",
        "kimi-for-coding",
      ),
      true,
    ),
  },
  {
    name: "DeepSeek",
    category: "cn_official",
    websiteUrl: "https://platform.deepseek.com",
    icon: "deepseek",
    iconColor: "#1E88E5",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "deepseek",
        "https://api.deepseek.com",
        "deepseek-v4-flash",
      ),
      true,
    ),
  },
  {
    name: "Zhipu GLM",
    category: "cn_official",
    websiteUrl: "https://open.bigmodel.cn",
    icon: "zhipu",
    iconColor: "#0F62FE",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "zhipu_glm",
        "https://open.bigmodel.cn/api/coding/paas/v4",
        "glm-5.2",
      ),
      true,
    ),
  },
  {
    name: "火山 Agentplan",
    category: "cn_official",
    websiteUrl: "https://www.volcengine.com/product/ark",
    icon: "huoshan",
    iconColor: "#3370FF",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "ark_agentplan",
        "https://ark.cn-beijing.volces.com/api/coding/v3",
        "ark-code-latest",
      ),
      true,
    ),
  },
  {
    name: "DouBaoSeed",
    category: "cn_official",
    websiteUrl: "https://console.volcengine.com/ark",
    icon: "doubao",
    iconColor: "#3370FF",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "doubaoseed",
        "https://ark.cn-beijing.volces.com/api/v3",
        "doubao-seed-2-1-pro-260628",
      ),
      true,
    ),
  },
  {
    name: "百度千帆",
    category: "cn_official",
    websiteUrl: "https://cloud.baidu.com/product/qianfan",
    icon: "baidu",
    iconColor: "#2932E1",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "qianfan_coding",
        "https://qianfan.baidubce.com/v2/coding",
        "qianfan-code-latest",
      ),
      true,
    ),
  },
  {
    name: "阿里百炼",
    category: "cn_official",
    websiteUrl: "https://bailian.console.aliyun.com",
    icon: "bailian",
    iconColor: "#624AFF",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "bailian",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        "qwen3-coder-plus",
      ),
      true,
    ),
  },
  {
    name: "StepFun",
    category: "cn_official",
    websiteUrl: "https://platform.stepfun.com/step-plan",
    icon: "stepfun",
    iconColor: "#16D6D2",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "stepfun",
        "https://api.stepfun.com/step_plan/v1",
        "step-3.7-flash",
      ),
      true,
    ),
  },
  {
    name: "MiniMax",
    category: "cn_official",
    websiteUrl: "https://platform.minimaxi.com",
    icon: "minimax",
    iconColor: "#FF6B6B",
    settingsConfig: codexSnapshot(
      thirdPartyConfig("minimax", "https://api.minimaxi.com/v1", "MiniMax-M3"),
      true,
    ),
  },
  {
    name: "小米 MiMo",
    category: "cn_official",
    websiteUrl: "https://platform.xiaomimimo.com",
    icon: "xiaomimimo",
    iconColor: "#000000",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "xiaomi_mimo",
        "https://api.xiaomimimo.com/v1",
        "mimo-v2.5-pro",
      ),
      true,
    ),
  },

  // ── 热门聚合 4 ──
  {
    name: "SiliconFlow",
    category: "aggregator",
    websiteUrl: "https://siliconflow.cn",
    icon: "siliconflow",
    iconColor: "#6E29F6",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "siliconflow",
        "https://api.siliconflow.cn/v1",
        "Pro/MiniMaxAI/MiniMax-M2.7",
      ),
      true,
    ),
  },
  {
    name: "OpenRouter",
    category: "aggregator",
    websiteUrl: "https://openrouter.ai",
    icon: "openrouter",
    iconColor: "#6566F1",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "openrouter",
        "https://openrouter.ai/api/v1",
        "gpt-5.6-sol",
      ),
      true,
    ),
  },
  {
    name: "ModelScope",
    category: "aggregator",
    websiteUrl: "https://modelscope.cn",
    icon: "modelscope",
    iconColor: "#624AFF",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "modelscope",
        "https://api-inference.modelscope.cn/v1",
        "ZhipuAI/GLM-5.1",
      ),
      true,
    ),
  },
  {
    name: "Novita AI",
    category: "aggregator",
    websiteUrl: "https://novita.ai",
    icon: "novita",
    iconColor: "#000000",
    settingsConfig: codexSnapshot(
      thirdPartyConfig(
        "novita",
        "https://api.novita.ai/openai/v1",
        "zai-org/glm-5.1",
      ),
      true,
    ),
  },
]
