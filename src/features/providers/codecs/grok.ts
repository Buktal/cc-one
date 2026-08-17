// Grok 应用的 settingsConfig codec：命名 profile TOML 的文本级读写。
//
// Grok settingsConfig 形状：`{"config": "<TOML 文本>"}`。config 即
// ~/.grok/config.toml 的受控片段——`[model.cc-one]` profile 块（写盘时整块替换
// + 设 models.default）。空 config = 登录态版（官方，Grok CLI 回落自带 xAI
// OAuth）。表单是纯 TOML 编辑器——api_key / model / base_url 等字段都在 TOML
// 内，前端无 TOML 解析器，故不拆结构化字段（与 codex 的 JSON-auth 抽离不同）。

type GrokConfig = { config: string }

/** 解析 Grok settingsConfig 文本为 `{config}`。宽容：空 / 垃圾 / 非对象 →
 *  `{config:""}`；非字符串 config 当 "" 处理——表单遇到手改坏的快照也不崩，
 *  写回时按归一结果继续。 */
export function parseGrokConfig(text: string): GrokConfig {
  if (!text) return { config: "" }
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { config: "" }
    }
    const obj = parsed as { config?: unknown }
    return { config: typeof obj.config === "string" ? obj.config : "" }
  } catch {
    return { config: "" }
  }
}

/** 读 Grok settingsConfig 的 `config`（config.toml 的 TOML 文本，即
 *  `[model.cc-one]` profile 块）。 */
export function grokConfigToml(text: string): string {
  return parseGrokConfig(text).config
}

/** 把 TOML 文本写入 Grok settingsConfig 的 `config` 字段（受控合并的目标值
 *  就是这里整段 TOML）。与 codex 的 withCodexConfigToml 同一「先 parse 再
 *  stringify」形态。 */
export function withGrokConfigToml(text: string, toml: string): string {
  const parsed = parseGrokConfig(text)
  return JSON.stringify({ ...parsed, config: toml }, null, 2)
}
