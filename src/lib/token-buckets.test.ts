// Tests for the TokenBuckets caliber（架构审查候选⑨）: 池定义、两种 binding
// 行形的适配与复合总量，全部纯函数直测生产路径。

import { describe, expect, it } from "vitest"

import { sumBuckets, tokenBuckets, totalTokensOf } from "./token-buckets"

describe("sumBuckets", () => {
  it("totals exactly the four buckets", () => {
    expect(
      sumBuckets({
        input: 30,
        output: 99,
        cache_creation: 10,
        cache_read: 60,
      }),
    ).toBe(199)
  })

  it("all-zero buckets total zero (no NaN/Guard drift)", () => {
    expect(
      sumBuckets({ input: 0, output: 0, cache_creation: 0, cache_read: 0 }),
    ).toBe(0)
  })
})

describe("tokenBuckets", () => {
  it("adapts flat *_tokens rows (SessionStatsRow / SessionUsageRow shape)", () => {
    expect(
      tokenBuckets({
        input_tokens: 1,
        output_tokens: 2,
        cache_creation_tokens: 3,
        cache_read_tokens: 4,
      }),
    ).toEqual({ input: 1, output: 2, cache_creation: 3, cache_read: 4 })
  })

  it("adapts nested token-pack rows (UsageLogRow shape)", () => {
    expect(
      tokenBuckets({
        tokens: { input: 5, output: 6, cache_creation: 7, cache_read: 8 },
      }),
    ).toEqual({ input: 5, output: 6, cache_creation: 7, cache_read: 8 })
  })
})

describe("totalTokensOf", () => {
  it("composes adaptation + summation for both shapes", () => {
    expect(
      totalTokensOf({
        input_tokens: 100,
        output_tokens: 20,
        cache_creation_tokens: 10,
        cache_read_tokens: 70,
      }),
    ).toBe(200)
    expect(
      totalTokensOf({
        tokens: { input: 1, output: 2, cache_creation: 3, cache_read: 4 },
      }),
    ).toBe(10)
  })
})
