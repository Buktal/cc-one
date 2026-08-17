// Gemini 应用的 settingsConfig codec：env 整块写 .env，config 合并进
// settings.json。
//
// Gemini settingsConfig 形状：`{"env": {...}, "config"?: {...}}`。`env` 整块
// 替换 ~/.gemini/.env；`config` 合并进 ~/.gemini/settings.json 的受控字段。
// env 含 `GEMINI_API_KEY` → API Key 版（selectedType="gemini-api-key"），
// 否则 → 登录态版（selectedType="oauth"，保留 Google 登录）。`config` 在
// 这里作为原始 JSON 文本返回，便于编辑器编辑。

type GeminiConfig = { env: Record<string, string>; config: string }

/** 解析 Gemini settingsConfig 文本为 `{env, config}`。config 字段以原始 JSON
 *  文本返回（`config ? JSON.stringify(config) : ""`），便于编辑器编辑与回写。
 *  宽容：空 / 垃圾 / 非对象 → `{env:{}, config:""}`；非对象 env 当 `{}`、非
 *  对象 config 当 `""` 处理——表单遇到手改坏的快照也不崩。 */
export function parseGeminiConfig(text: string): GeminiConfig {
  if (!text) return { env: {}, config: "" }
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return { env: {}, config: "" }
    }
    const obj = parsed as { env?: unknown; config?: unknown }
    const env =
      obj.env !== null && typeof obj.env === "object" && !Array.isArray(obj.env)
        ? Object.fromEntries(
            Object.entries(obj.env as Record<string, unknown>).filter(
              (entry): entry is [string, string] =>
                typeof entry[1] === "string",
            ),
          )
        : {}
    const config =
      obj.config !== null &&
      typeof obj.config === "object" &&
      !Array.isArray(obj.config)
        ? JSON.stringify(obj.config)
        : ""
    return { env, config }
  } catch {
    return { env: {}, config: "" }
  }
}

/** 读 Gemini settingsConfig 的 `env.GEMINI_API_KEY`（API Key 版的判据 + 凭据）。 */
export function geminiApiKey(text: string): string {
  return parseGeminiConfig(text).env.GEMINI_API_KEY ?? ""
}

/** 读 Gemini settingsConfig 的 `env.GEMINI_MODEL`（主模型名）。 */
export function geminiModel(text: string): string {
  return parseGeminiConfig(text).env.GEMINI_MODEL ?? ""
}

/** 读 Gemini settingsConfig 的 `env.GOOGLE_GEMINI_BASE_URL`（端点）。 */
export function geminiBaseUrl(text: string): string {
  return parseGeminiConfig(text).env.GOOGLE_GEMINI_BASE_URL ?? ""
}

/** 把 patch 的键合并进 Gemini settingsConfig 的 env：值为空串则删除该键
 *  （`GEMINI_API_KEY` 删除即回归登录态版），非空则写入 / 覆盖。保留 config
 *  字段与其余 env 键不动。 */
export function withGeminiEnv(
  text: string,
  patch: Record<string, string>,
): string {
  const { env, config } = parseGeminiConfig(text)
  const next = { ...env }
  for (const [key, value] of Object.entries(patch)) {
    if (value) next[key] = value
    else delete next[key]
  }
  // 配置回写以 parseGeminiConfig 解出的 JSON 文本为准——保留原始结构而非
  // 重新序列化，避免合法 config 被空对象覆盖。
  const configObj: Record<string, unknown> | undefined = config
    ? (JSON.parse(config) as Record<string, unknown>)
    : undefined
  return JSON.stringify(
    configObj ? { env: next, config: configObj } : { env: next },
    null,
    2,
  )
}

/** 把 JSON 文本写入 Gemini settingsConfig 的 `config` 字段。空串 → 删除
 *  config 键；非空 → 尝试 JSON.parse，非法 → 原样不动返回原 text（编辑器
 *  正在敲半截 JSON 时不会被吞）。保留 env 字段不动。 */
export function withGeminiConfigJson(text: string, configJson: string): string {
  const { env } = parseGeminiConfig(text)
  if (!configJson) {
    return JSON.stringify({ env }, null, 2)
  }
  try {
    const parsed = JSON.parse(configJson) as unknown
    if (
      parsed === null ||
      typeof parsed !== "object" ||
      Array.isArray(parsed)
    ) {
      return text
    }
    return JSON.stringify(
      { env, config: parsed as Record<string, unknown> },
      null,
      2,
    )
  } catch {
    return text
  }
}
