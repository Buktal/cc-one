// Token 四桶与「总量」的唯一口径（架构审查候选⑨）。Rust TokenCounts（u32 四
// 件组）的 JS 镜像：凡「input+output+cache_creation+cache_read 相加」都必须
// 经由这里的 sumBuckets，不准在各组件重抄求和式——漏一桶/字段名写错这类漂移
// 只能活在这一处文件里，排查总量/占比不一致时坐标唯此一间。展示侧同理：
// 桶序 ↔ 展示色 ↔ 文案键尾段的唯一手写处是文末的 BUCKET_DISPLAY 名册。

export interface TokenBuckets {
  input: number
  output: number
  cache_creation: number
  cache_read: number
}

/** 总量 = 四桶之和。池定义唯一定义处（命中率池含不含 output 的答案不在这——
 *  那 cacheable pool 属于派生口径，见 sessions derive 的 tokensHitRate）。 */
export function sumBuckets(t: TokenBuckets): number {
  return t.input + t.output + t.cache_creation + t.cache_read
}

// 后端多数 binding 行把四桶平铺（*_tokens 后缀：SessionStatsRow /
// SessionUsageRow / TrendPoint…），用量日志行则嵌套 tokens 包（UsageLogRow）。
interface FlatBucketRow {
  input_tokens: number
  output_tokens: number
  cache_creation_tokens: number
  cache_read_tokens: number
}

interface PackBucketRow {
  tokens: TokenBuckets
}

/** 行 → 四桶适配：两种 binding 行形各取所依，调用方不再按字段名拼装。 */
export function tokenBuckets(row: FlatBucketRow | PackBucketRow): TokenBuckets {
  return "tokens" in row
    ? row.tokens
    : {
        input: row.input_tokens,
        output: row.output_tokens,
        cache_creation: row.cache_creation_tokens,
        cache_read: row.cache_read_tokens,
      }
}

/** 单行的总 Token 量（tokenBuckets ∘ sumBuckets 的常用复合）。 */
export function totalTokensOf(row: FlatBucketRow | PackBucketRow): number {
  return sumBuckets(tokenBuckets(row))
}

// -------------------------------- 展示名册 --------------------------------

/** TokenBuckets 的桶键（四桶算术键，stats-rail 等按它从桶对象取值）。 */
export type TokenBucketKey = keyof TokenBuckets

/** 平铺 binding 行（TrendPoint / UsageStats / SessionStatsRow…）上的四桶
 *  字段名（`*_tokens` 后缀族，token hero / 趋势图按它取数）。 */
export type BucketStatKey = `${TokenBucketKey}_tokens`

/** 名册行：桶 → 展示色 → 文案键尾段。 */
export interface BucketDisplayEntry {
  /** 算术桶键（TokenBuckets 字段名）。 */
  bucket: TokenBucketKey
  /** 展示色：B 级语义 chart token 的引用（定义唯 index.css，名册只引用、
   *  不改定义；色值随皮肤走）。 */
  cssVar: `var(--chart-${string})`
  /** 文案键尾段：usage 域 `usage.tokens.${suffix}` 与 sessions 域
   *  `sessions.stats.bucket.${suffix}` 两域键不同构（同义文案措辞各异，
   *  如 en 的「Cache hit」vs「Cache read」），名册只持两域共用的尾段，
   *  域前缀由消费点拼接。 */
  suffix: "input" | "output" | "cacheCreation" | "cacheRead"
}

/** Token 四桶「展示名册」（架构审查Ⅳ候选⑪）：桶 → 色 → 文案尾段，
 *  **固定序即展示序契约**——token hero 堆叠条 / usage 趋势图线序 / 会话
 *  统计条形共用同一序。色与文案键曾在 usage / sessions 两域四处手抄
 *  （token-hero SEGMENTS、trend-chart BUCKETS + chartConfig、stats-rail
 *  BUCKET_COLORS + 内联 segments），新增一桶从「六处同改」收敛为
 *  「本表一处 + 三语言包各一」。 */
export const BUCKET_DISPLAY: readonly BucketDisplayEntry[] = [
  { bucket: "input", cssVar: "var(--chart-input)", suffix: "input" },
  { bucket: "output", cssVar: "var(--chart-output)", suffix: "output" },
  {
    bucket: "cache_creation",
    cssVar: "var(--chart-cache-create)",
    suffix: "cacheCreation",
  },
  {
    bucket: "cache_read",
    cssVar: "var(--chart-cache-read)",
    suffix: "cacheRead",
  },
]

/** 桶键 → 展示色的取色索引：按 TokenBuckets 字段取色（无序消费）的场景
 *  用它，不必在名册数组里 find。由名册派生，非第二份手抄（如会话统计
 *  命中率沿用 cache_read 的语义色）。 */
export const BUCKET_COLOR = Object.fromEntries(
  BUCKET_DISPLAY.map((b) => [b.bucket, b.cssVar]),
) as Record<TokenBucketKey, BucketDisplayEntry["cssVar"]>
