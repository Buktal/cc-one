// Token 四桶与「总量」的唯一口径（架构审查候选⑨）。Rust TokenCounts（u32 四
// 件组）的 JS 镜像：凡「input+output+cache_creation+cache_read 相加」都必须
// 经由这里的 sumBuckets，不准在各组件重抄求和式——漏一桶/字段名写错这类漂移
// 只能活在这一处文件里，排查总量/占比不一致时坐标唯此一间。

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
