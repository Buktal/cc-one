// 切换前必填项检查（`providerMissingRequired`）：按 provider 归属应用取必填
// 键集，与其预设的占位符键一致，报告缺失的部分（端点、API key、未物化的模板
// 变量）。各应用的取值走对应 codec 的解析（`parseCodexConfig` 等）——本模块
// 只做「缺不缺」的判定，不重复任何解析逻辑。

import type { Provider } from "@/types/generated/bindings"
import { providerApiKey, providerEndpoint } from "./codecs/claude"
import { parseCodexConfig } from "./codecs/codex"
import { parseGeminiConfig } from "./codecs/gemini"
import { grokConfigToml } from "./codecs/grok"
import {
  openCodeApiKey,
  openCodeOptionsOf,
  parseOpenCodeEntry,
} from "./codecs/opencode"
import { extractTemplateVars } from "./derive"

/** 宽容读 TOML 文本里首个 `key = "value"` 字符串赋值（codex/grok 的
 *  settingsConfig 是 preset 形状的机器生成 TOML；缺键 → null）。切换前必填项
 *  检查用——advisory 检查，正式 TOML 解析在后端（write 层）。 */
function tomlStringField(toml: string, key: string): string | null {
  const m = toml.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"]*)"`, "m"))
  return m ? m[1] : null
}

/** 必填值缺失判定：`null` = 键不存在（该应用此版本不启用此必填项，如 codex
 *  登录态版无 auth key）；存在但空白 = 占位未填 → 缺失。第三方分类
 *  （≠ official / cloud_provider）对端点与 key 一律要求非空（与 claude 同一
 *  规则）；official 分类只在「占位键存在但为空」时告警（如 codex/gemini 的
 *  官方 API Key 版预设带空占位，登录态版不带键则不告警）。 */
function missingIf(thirdParty: boolean, value: string | null): boolean {
  if (value === null) return thirdParty
  return !value.trim()
}

/** 必填检查的缺失项枚举（`providers.switchConfirm.missing.*` 动态键的取值
 *  域——见 i18n/dynamic-keys.ts 的键集护栏）。 */
export type MissingRequiredField = "endpoint" | "apiKey" | "templateVars"

/** 切换前必填项检查（按 provider 归属应用取必填键集，与其预设的占位符键一
 *  致）：缺失的部分（端点、API key、未物化的模板变量）。
 *  - claude：端点取 `ANTHROPIC_BASE_URL`、key 取 AUTH_TOKEN 优先后 API_KEY
 *    （表单收集语义）。官方 / 云厂商预设不要求端点/key（Claude Official 走
 *    默认端点，Bedrock 用模板变量认证）。
 *  - codex：key 取 `auth.OPENAI_API_KEY`、端点取 config TOML 的 `base_url`
 *    （model_providers 表内）；登录态版两者皆无 → 不告警。
 *  - gemini：key 取 `env.GEMINI_API_KEY`、端点取 `env.GOOGLE_GEMINI_BASE_URL`。
 *  - grok：key/端点取 config TOML `[model."cc-one"]` 的 `api_key` / `base_url`。
 *  - opencode：附加模式无登录态版——`options.apiKey` 一律要求非空；端点
 *    `options.baseURL` 只在占位存在但为空时告警（缺省 = npm SDK 自带端点）。
 *  模板变量残留检查对全部应用生效。空数组 = 没有缺失。 */
export function providerMissingRequired(
  provider: Provider,
): MissingRequiredField[] {
  const missing: MissingRequiredField[] = []
  const thirdParty =
    provider.category !== "official" && provider.category !== "cloud_provider"
  const app = provider.app ?? "claude"
  switch (app) {
    case "claude": {
      if (thirdParty && !providerEndpoint(provider).trim()) {
        missing.push("endpoint")
      }
      if (thirdParty && !providerApiKey(provider).trim()) {
        missing.push("apiKey")
      }
      break
    }
    case "codex": {
      const { auth, config } = parseCodexConfig(provider.settingsConfig)
      if (missingIf(thirdParty, tomlStringField(config, "base_url"))) {
        missing.push("endpoint")
      }
      const key = "OPENAI_API_KEY" in auth ? (auth.OPENAI_API_KEY ?? "") : null
      if (missingIf(thirdParty, key)) missing.push("apiKey")
      break
    }
    case "gemini": {
      const { env } = parseGeminiConfig(provider.settingsConfig)
      if (
        missingIf(
          thirdParty,
          "GOOGLE_GEMINI_BASE_URL" in env
            ? (env.GOOGLE_GEMINI_BASE_URL ?? "")
            : null,
        )
      ) {
        missing.push("endpoint")
      }
      if (
        missingIf(
          thirdParty,
          "GEMINI_API_KEY" in env ? (env.GEMINI_API_KEY ?? "") : null,
        )
      ) {
        missing.push("apiKey")
      }
      break
    }
    case "grok": {
      const toml = grokConfigToml(provider.settingsConfig)
      if (missingIf(thirdParty, tomlStringField(toml, "base_url"))) {
        missing.push("endpoint")
      }
      if (missingIf(thirdParty, tomlStringField(toml, "api_key"))) {
        missing.push("apiKey")
      }
      break
    }
    case "opencode": {
      // opencode 无登录态版：key 一律必填；baseURL 占位空串才告警（缺省走
      // npm SDK 自带端点）。
      const options = openCodeOptionsOf(
        parseOpenCodeEntry(provider.settingsConfig),
      )
      if (!openCodeApiKey(provider.settingsConfig).trim()) {
        missing.push("apiKey")
      }
      const baseURL = options.baseURL
      if (typeof baseURL === "string" && !baseURL.trim()) {
        missing.push("endpoint")
      }
      break
    }
  }
  if (extractTemplateVars(provider.settingsConfig).length > 0) {
    missing.push("templateVars")
  }
  return missing
}
