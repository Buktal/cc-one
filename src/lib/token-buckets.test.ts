// Tests for the TokenBuckets caliber（架构审查候选⑨）: 池定义、两种 binding
// 行形的适配与复合总量，全部纯函数直测生产路径。末组是展示名册 BUCKET_DISPLAY
// 的 parity 红绿灯（架构审查Ⅳ候选⑪，照 presets.test.ts 的镜像常量模式）。

import { describe, expect, it } from "vitest"

import en from "@/locales/en.json"
import ja from "@/locales/ja.json"
import zh from "@/locales/zh.json"

import {
  BUCKET_COLOR,
  BUCKET_DISPLAY,
  sumBuckets,
  type TokenBuckets,
  tokenBuckets,
  tokensHitRate,
  totalTokensOf,
} from "./token-buckets"

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

describe("tokensHitRate", () => {
  it("derives cache_read / (input + cache_creation + cache_read)", () => {
    expect(
      tokensHitRate({
        input: 30,
        output: 99,
        cache_creation: 10,
        cache_read: 60,
      }),
    ).toBeCloseTo(60 / 100)
  })

  it("null when the cacheable pool is empty (no usage)", () => {
    expect(
      tokensHitRate({ input: 0, output: 5, cache_creation: 0, cache_read: 0 }),
    ).toBeNull()
  })
})

// ---- 展示名册 parity（架构审查Ⅳ候选⑪）----
//
// BUCKET_DISPLAY 是「桶序 ↔ 展示色 ↔ 文案键尾段」的唯一手写处，token hero /
// 趋势图 / 会话统计的四处旧手抄已全部改为从它派生——这里用镜像常量当红绿灯：
// 漏桶、换序、色与文案尾段拼错（含桶↔键错配，存在性检查抓不住的那种）都在
// 此处红，而不是等四张卡各唱各的才靠人眼发现。

/** 镜像：名册应有的逐行内容（顺序敏感）。改名册必须同改这里，漂移在
 *  review 里一眼可见、在 CI 里直接红。 */
const ROSTER_MIRROR = [
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
] as const

describe("BUCKET_DISPLAY 展示名册 parity", () => {
  it("与镜像逐行相等（长度 / 序 / 每列值，顺序敏感）", () => {
    expect([...BUCKET_DISPLAY]).toEqual(ROSTER_MIRROR.map((r) => ({ ...r })))
  })

  it("恰好覆盖 TokenBuckets 的每一个桶（算术加桶必须同步名册）", () => {
    // satisfies 令该样本在编译期跟随 TokenBuckets 形状，接口加桶后这里
    // 先编译红，运行期再断言名册跟上——两道闸都指向名册这一处修改点。
    const arithmetic = {
      input: 0,
      output: 0,
      cache_creation: 0,
      cache_read: 0,
    } satisfies TokenBuckets
    expect(BUCKET_DISPLAY.map((b) => b.bucket)).toEqual(Object.keys(arithmetic))
  })

  it("cssVar 互不重复且都是 chart token 引用", () => {
    const vars = BUCKET_DISPLAY.map((b) => b.cssVar)
    expect(new Set(vars).size).toBe(vars.length)
    for (const v of vars) {
      expect(v).toMatch(/^var\(--chart-[a-z-]+\)$/)
    }
  })

  it("两域文案键（usage.tokens.* / sessions.stats.bucket.*）在 zh/en/ja 全部存在", () => {
    // 名册只持尾段、域前缀由消费点拼接，整键从此没有静态字面量可查——
    // 尾段拼错的语言包缺键表现为裸键名，靠这里的三语存在性断言拦截。
    const locales = { zh, en, ja }
    for (const prefix of ["usage.tokens", "sessions.stats.bucket"]) {
      for (const entry of BUCKET_DISPLAY) {
        const key = `${prefix}.${entry.suffix}`
        for (const [name, locale] of Object.entries(locales)) {
          expect(
            key in locale,
            `${key} 缺于 ${name}（名册尾段 ${entry.suffix} 拼错或语言包缺键）`,
          ).toBe(true)
        }
      }
    }
  })

  it("BUCKET_COLOR 与名册同源（桶键 → 色一一对应）", () => {
    for (const b of BUCKET_DISPLAY) {
      expect(BUCKET_COLOR[b.bucket]).toBe(b.cssVar)
    }
  })
})
