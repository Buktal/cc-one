// 通用配置片段（snippet）域：子集判定、凭据键拦截、候选分组。
//
// `snippetMissingKeys` 是「片段子集判定」：对比片段与某份 settingsConfig，
// 报告片段里存在、而配置里缺失的受控字段——这些是切换写盘时片段实际会补上
// 的部分。写盘合并的权威在 Rust（provider::snippet 只认受控字段），这里只
// 为 UI 提示（片段卡片显示当前激活供应商会从片段得到什么）维护同一份受控
// 字段清单，**必须与后端 `provider::live::CONTROLLED_FIELDS` 保持同步**。
// 凭据键判定（`isSensitiveConfigKey` 一族）同样镜像后端
// `provider::snippet::is_sensitive_config_key`，**逐字一致**（ADR-0010：
// 前后端判定必须一致，否则后端拦了、前端还允许写回）。

/** 写盘路径承认的受控字段（镜像后端常量；仅供 UI 提示使用，写盘权威在后端）。 */
const CONTROLLED_FIELDS = [
  "env",
  "includeCoAuthoredBy",
  "attribution",
  "effortLevel",
  "enabledPlugins",
  "skipWebFetchPreflight",
] as const

/** 片段子集判定的严格解析：空串 → `{}`；非空但非法 JSON 或非对象 → `null`
 *  ——解析不了的输入没法可靠判定缺失，调用方对 `null` 一律报 `[]`，不误导
 *  （与 `parseSettingsConfig` 的宽容契约不同：它把垃圾吞成 `{}` 以让表单
 *  不崩，而这里空与垃圾必须分得清，否则垃圾配置会误报「片段将补 X」）。 */
function parseSnippetInput(text: string): Record<string, unknown> | null {
  if (!text) return {}
  try {
    const parsed: unknown = JSON.parse(text)
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return null
    }
    return parsed as Record<string, unknown>
  } catch {
    return null
  }
}

/** env 非对象（手写垃圾）按空对象处理——与后端合并语义一致：非对象 env
 *  被跳过、不参与键级判定。 */
function envRecordOf(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
}

/** 片段对 settingsConfig 的补充（子集判定）：片段里出现、而配置里缺失的
 *  受控字段键——这些是切换写盘时片段会补上的部分。`env` 按键级判定：片段
 *  env 有任一键缺失即报告 `env`。非受控键不算（写盘合并时被忽略，报了是
 *  误导）。配置/片段为空 → `[]`；非空但解析不了或不是对象 → `[]`。 */
export function snippetMissingKeys(
  configText: string,
  snippetText: string,
): string[] {
  const config = parseSnippetInput(configText)
  const snippet = parseSnippetInput(snippetText)
  if (config === null || snippet === null) return []
  const missing: string[] = []
  for (const key of CONTROLLED_FIELDS) {
    if (key === "env") {
      const configEnv = envRecordOf(config.env)
      const snippetEnv = envRecordOf(snippet.env)
      if (Object.keys(snippetEnv).some((k) => !(k in configEnv))) {
        missing.push("env")
      }
    } else if (key in snippet && !(key in config)) {
      missing.push(key)
    }
  }
  return missing
}

/** gemini 片段子集判定：片段 env 键 vs 配置 env 键——gemini 的合并层在
 *  settings_config（片段 env 键级补缺失、供应商赢，ADR-0010），返回片段里有、
 *  配置里没有的 env 键。与 claude 版同契约：解析不了 / 非对象 → `[]`（不误导）。 */
export function geminiSnippetMissingKeys(
  configText: string,
  snippetText: string,
): string[] {
  const config = parseSnippetInput(configText)
  const snippet = parseSnippetInput(snippetText)
  if (config === null || snippet === null) return []
  const configEnv = envRecordOf(config.env)
  const snippetEnv = envRecordOf(snippet.env)
  return Object.keys(snippetEnv).filter((k) => !(k in configEnv))
}

/** 现有片段里已覆盖的键集合（T6「提取为通用片段」候选过滤用，ADR-0012「且片段
 *  缺」条件）：按片段编辑器语言解析片段内容——JSON = 顶层键 + env 内键；
 *  TOML = 顶层表 / 顶层标量键（行级容错解析，坏行忽略，能解析多少算多少）。
 *  语言取自 app-profiles 的 snippetSupportLanguage（TOML = 写盘层合并应用），
 *  本函数不再持有 app 身份分支。解析不了的片段 → 空集（无键被覆盖，候选全
 *  保留）。 */
export function snippetCoveredKeys(
  language: "json" | "toml",
  snippetText: string,
): Set<string> {
  const covered = new Set<string>()
  if (language === "toml") {
    let inTable = false
    for (const line of snippetText.split("\n")) {
      const trimmed = line.trim()
      if (!trimmed || trimmed.startsWith("#")) continue
      const table = trimmed.match(/^\[([^.\]]+)(?:\.|\])/)
      if (table) {
        inTable = true
        covered.add(table[1])
        continue
      }
      const keyValue = trimmed.match(/^([A-Za-z0-9_-]+)\s*=/)
      if (keyValue && !inTable) covered.add(keyValue[1])
    }
    return covered
  }
  const obj = parseSnippetInput(snippetText)
  if (!obj) return covered
  for (const key of Object.keys(obj)) covered.add(key)
  for (const key of Object.keys(envRecordOf(obj.env))) covered.add(key)
  return covered
}

/** 可共享配置候选的三组人话分组（导入后「提取为通用片段」提示用，前端展示
 *  只影响文案结构、不改变提取内容）：含 BASE_URL 的键 → 端点组；含 MODEL 的
 *  键 → 模型组；其余 → 行为开关组。组内保持原候选顺序。 */
export function groupSnippetCandidates(keys: string[]): {
  endpoint: string[]
  model: string[]
  behavior: string[]
} {
  const endpoint: string[] = []
  const model: string[] = []
  const behavior: string[] = []
  for (const k of keys) {
    const upper = k.toUpperCase()
    if (upper.includes("BASE_URL")) endpoint.push(k)
    else if (upper.includes("MODEL")) model.push(k)
    else behavior.push(k)
  }
  return { endpoint, model, behavior }
}

/** 模型组候选的展示排序：把 `X_MODEL` 与其 `X_MODEL_NAME` 配成对、MODEL 在前
 *  NAME 紧随（等宽字体下前缀对齐，用户一眼认出「几个角色的默认模型与显示名」）。
 *  无配对的键（如 ANTHROPIC_MODEL、CLAUDE_CODE_SUBAGENT_MODEL）保留原相对
 *  位置。NAME 先于 MODEL 出现时也把 NAME 挪到 MODEL 后配对。 */
export function pairModelNameKeys(keys: string[]): string[] {
  // X_MODEL → X_MODEL_NAME 的形态配对（只对真存在配对的键登记）。
  const nameOf = new Map<string, string>()
  for (const k of keys) {
    if (!k.endsWith("_MODEL_NAME")) continue
    const modelKey = k.slice(0, -"_NAME".length)
    if (keys.includes(modelKey)) nameOf.set(modelKey, k)
  }
  const out: string[] = []
  const emitted = new Set<string>()
  // 配对 NAME 遇到即跳过（由 MODEL 处输出）——NAME 先于 MODEL 出现也成立。
  const pairedNames = new Set(nameOf.values())
  for (const k of keys) {
    if (emitted.has(k) || pairedNames.has(k)) continue
    const nameKey = nameOf.get(k)
    if (nameKey) {
      out.push(k, nameKey)
      emitted.add(nameKey)
    } else {
      out.push(k)
    }
  }
  return out
}

// ---- 通用片段凭据键判定（claude / gemini 共用）----
//
// TS 镜像后端 `provider::snippet::is_sensitive_config_key`，**逐字一致**
// （ADR-0010：前后端判定必须一致，否则后端拦了、前端还允许写回）。三张表移植
// 自 CC-Switch，统一转大写后比较：EXACT 精确、SUFFIXES 后缀、CONTAINS 子串。
// 模式匹配覆盖整类凭据键而非枚举——枚举挡不住下一个 `*_API_KEY`（CC-Switch 曾
// 因只枚举两个固定键名，让 `GOOGLE_API_KEY` 漏进共享片段、被深合并进其它供应商
// env、随请求发往对方 base_url）。仅 claude / gemini 片段用（两者写 env 认证
// 通道）；codex / grok 片段写 mcp_servers（不经 LLM 端点），允许凭据，不用本判定。

/** 凭据键精确匹配表（转大写后全等）。与后端 EXACT 逐字一致。 */
const SENSITIVE_EXACT = [
  "APIKEY",
  "API_KEY",
  "TOKEN",
  "SECRET",
  "PASSWORD",
  "CREDENTIALS",
] as const

/** 凭据键后缀表（转大写后以此结尾）。与后端 SUFFIXES 逐字一致。 */
const SENSITIVE_SUFFIXES = [
  "_KEY",
  "_API_KEY",
  "_ACCESS_KEY",
  "_ACCESS_KEY_ID",
  "_KEY_ID",
  "_PRIVATE_KEY",
  "_APIKEY",
  "_ACCESSKEY",
  "_SECRETKEY",
  "_APITOKEN",
  "_AUTH_TOKEN",
  "_TOKEN",
  "_PAT",
  "_PWD",
  "_PASS",
  "_PASSPHRASE",
  "_CREDS",
] as const

/** 凭据键子串表（转大写后包含）。与后端 CONTAINS 逐字一致。 */
const SENSITIVE_CONTAINS = [
  "SECRET",
  "PASSWORD",
  "PASSWD",
  "CREDENTIAL",
  "PRIVATE_KEY",
  "BEARER_TOKEN",
] as const

/** 判定一个配置键名是否像凭据（API key / token / secret / password 等）。
 *  与后端 `is_sensitive_config_key` 对齐：三张表相同、统一转大写后比较。env 键
 *  均 ASCII，故 JS `toUpperCase` 与后端 `to_ascii_uppercase` 实际等价（非 ASCII
 *  键理论上有差异，但配置键名不存在该情况）。 */
export function isSensitiveConfigKey(name: string): boolean {
  const upper = name.toUpperCase()
  return (
    SENSITIVE_EXACT.some((e) => upper === e) ||
    SENSITIVE_SUFFIXES.some((s) => upper.endsWith(s)) ||
    SENSITIVE_CONTAINS.some((c) => upper.includes(c))
  )
}

/** 扫描片段对象（顶层 + env 子对象）的键，返回第一个凭据键的显示路径（如
 *  `GEMINI_API_KEY` 或 `env.GEMINI_API_KEY`），无则 null。与后端
 *  `reject_sensitive_keys` 同款扫描范围（顶层 + env.*）。 */
export function findSensitiveConfigKey(
  obj: Record<string, unknown>,
): string | null {
  for (const key of Object.keys(obj)) {
    if (isSensitiveConfigKey(key)) return key
  }
  const env = obj.env
  if (env && typeof env === "object" && !Array.isArray(env)) {
    for (const key of Object.keys(env as Record<string, unknown>)) {
      if (isSensitiveConfigKey(key)) return `env.${key}`
    }
  }
  return null
}

/** gemini 端点键（决定凭据发往何处；共享会把认证引到错误端点，归供应商）。 */
const GEMINI_ENDPOINT_KEY = "GOOGLE_GEMINI_BASE_URL"

/** gemini 片段的保存前问题（前端提示用，advisory——实际拒绝仍由后端）：凭据键
 *  （顶层 + env.*，与后端 `reject_sensitive_keys` 同款扫描范围）、env 下的端点键
 *  GOOGLE_GEMINI_BASE_URL、或 env 以外的顶层键（合并层只认 env 子对象，扁平键/
 *  其它顶层键零效果——与后端 `validate_snippet` gemini 分支同判）→ 返回该键的
 *  显示路径；无问题 / 解析不了的草稿 → null。env 值非字符串或空由后端校验
 *  兜底（JSON 编辑器 + 格式化已降低这类误写）。 */
export function geminiSnippetIssue(snippetText: string): string | null {
  const obj = parseSnippetInput(snippetText)
  if (!obj) return null
  const credential = findSensitiveConfigKey(obj)
  if (credential) return credential
  for (const key of Object.keys(obj)) {
    if (key !== "env") return key
  }
  const env = obj.env
  if (
    env &&
    typeof env === "object" &&
    !Array.isArray(env) &&
    GEMINI_ENDPOINT_KEY in (env as Record<string, unknown>)
  ) {
    return `env.${GEMINI_ENDPOINT_KEY}`
  }
  return null
}
