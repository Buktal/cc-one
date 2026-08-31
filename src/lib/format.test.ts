import dayjs from "dayjs"
import type { TFunction } from "i18next"
import { describe, expect, it } from "vitest"

import "@/i18n/languages"
import {
  dateInputToDay,
  formatCost,
  formatCostAmount,
  formatCostPrecise,
  formatCount,
  formatDay,
  formatDuration,
  formatDurationLabel,
  formatInt,
  formatMetricLine,
  formatMetricSeg,
  formatPct,
  formatRatio,
  formatRelative,
  formatSegValue,
  formatSize,
  formatTime,
  formatTimeExact,
  formatTokens,
  spanLabelKey,
  spanMsOf,
  spanParts,
} from "@/lib/format"

describe("formatTokens", () => {
  it("treats nullish as 0", () => {
    expect(formatTokens(null)).toBe("0")
    expect(formatTokens(undefined)).toBe("0")
  })

  it("treats non-finite as 0", () => {
    expect(formatTokens(Number.NaN)).toBe("0")
    expect(formatTokens(Number.POSITIVE_INFINITY)).toBe("0")
  })

  it("leaves sub-1K numbers plain (no thousands grouping under 1000)", () => {
    expect(formatTokens(0)).toBe("0")
    expect(formatTokens(856)).toBe("856")
    expect(formatTokens(999)).toBe("999")
  })

  it("compacts to K/M/B and trims trailing zeros", () => {
    expect(formatTokens(1200)).toBe("1.2K")
    expect(formatTokens(3_610_000)).toBe("3.61M")
    expect(formatTokens(1_500_000_000)).toBe("1.5B")
  })
})

describe("formatCount (DSL 计数类)", () => {
  it("nullish / non-finite → 0", () => {
    expect(formatCount(null)).toBe("0")
    expect(formatCount(Number.NaN)).toBe("0")
  })

  it("below 10K stays the grouped integer", () => {
    expect(formatCount(0)).toBe("0")
    expect(formatCount(9_999)).toBe("9,999")
  })

  it("compacts only from 10K up, one decimal, trailing zero dropped", () => {
    expect(formatCount(10_000)).toBe("10K")
    expect(formatCount(24_670)).toBe("24.7K")
    expect(formatCount(1_500_000)).toBe("1.5M")
    expect(formatCount(2_000_000_000)).toBe("2B")
  })
})

describe("formatCostAmount", () => {
  it("nullish / non-finite → 0.0000", () => {
    expect(formatCostAmount(null)).toBe("0.0000")
    expect(formatCostAmount(undefined)).toBe("0.0000")
    expect(formatCostAmount(Number.NaN)).toBe("0.0000")
  })

  it("formats the plain 4-decimal amount, no currency symbol", () => {
    expect(formatCostAmount(1.7564)).toBe("1.7564")
    expect(formatCostAmount(0)).toBe("0.0000")
    expect(formatCostAmount(-1.5)).toBe("-1.5000")
  })
})

describe("formatCost (DSL 成本恒两位)", () => {
  it("nullish / non-finite → $0.00", () => {
    expect(formatCost(null)).toBe("$0.00")
    expect(formatCost(undefined)).toBe("$0.00")
    expect(formatCost(Number.NaN)).toBe("$0.00")
  })

  it("formats USD with exactly two decimals (trailing zero kept)", () => {
    expect(formatCost(1.7564)).toBe("$1.76")
    expect(formatCost(0)).toBe("$0.00")
    expect(formatCost(12)).toBe("$12.00")
    expect(formatCost(-1.5)).toBe("$-1.50")
  })
})

describe("formatCostPrecise (ledger surfaces)", () => {
  it("is the currency symbol prefixed to formatCostAmount (single source)", () => {
    expect(formatCostPrecise(0.0003)).toBe(`$${formatCostAmount(0.0003)}`)
    expect(formatCostPrecise(null)).toBe("$0.0000")
  })
})

describe("formatInt", () => {
  it("truncates and groups thousands", () => {
    expect(formatInt(1234567)).toBe("1,234,567")
    expect(formatInt(12.9)).toBe("12")
    expect(formatInt(null)).toBe("0")
  })
})

describe("formatPct", () => {
  it("maps a [0,1] ratio to a percent string", () => {
    expect(formatPct(0.902)).toBe("90.2%")
    expect(formatPct(0)).toBe("0.0%")
    expect(formatPct(null)).toBe("0.0%")
    expect(formatPct(Number.NaN)).toBe("0.0%")
  })

  it("keeps the trailing zero — one decimal always (DSL 占比/比率)", () => {
    expect(formatPct(0.96)).toBe("96.0%")
    expect(formatPct(0.032)).toBe("3.2%")
  })
})

describe("formatRatio (DSL 比率一位小数)", () => {
  it("formats a plain ratio with one decimal, trailing zero kept", () => {
    expect(formatRatio(2)).toBe("2.0")
    expect(formatRatio(12.34)).toBe("12.3")
  })

  it("nullish / non-finite → 0.0", () => {
    expect(formatRatio(null)).toBe("0.0")
    expect(formatRatio(Number.NaN)).toBe("0.0")
  })
})

describe("formatMetricSeg / formatSegValue / formatMetricLine (DSL 段)", () => {
  it("builds `标签 数量` and appends ` · 占比` only when a share is given", () => {
    expect(formatMetricSeg("输入", "96.37M", 0.96)).toBe("输入 96.37M · 96.0%")
    expect(formatMetricSeg("请求", "24.7K")).toBe("请求 24.7K")
    expect(formatMetricSeg("请求", "9,999", null)).toBe("请求 9,999")
  })

  it("formatSegValue is the label-less half (label rendered by the layout)", () => {
    expect(formatSegValue("1.83B", 0.617)).toBe("1.83B · 61.7%")
    expect(formatSegValue("406K")).toBe("406K")
    // 0% shows (0 share is a real share, not "no share").
    expect(formatSegValue("0", 0)).toBe("0 · 0.0%")
  })

  it("joins segments with the DSL separator", () => {
    expect(
      formatMetricLine([
        formatMetricSeg("请求", "24.7K"),
        formatMetricSeg("命中率", "96.0%"),
        formatMetricSeg("成本", "$12.34"),
      ]),
    ).toBe("请求 24.7K · 命中率 96.0% · 成本 $12.34")
  })
})

describe("formatDuration", () => {
  it("em-dash for nullish / non-positive / non-finite", () => {
    expect(formatDuration(null)).toBe("—")
    expect(formatDuration(undefined)).toBe("—")
    expect(formatDuration(0)).toBe("—")
    expect(formatDuration(-5)).toBe("—")
    expect(formatDuration(Number.NaN)).toBe("—")
  })

  it("sub-minute → seconds with one decimal", () => {
    expect(formatDuration(12_300)).toBe("12.3s")
    expect(formatDuration(999)).toBe("1.0s")
  })

  it(">= 1 minute → mSS format, zero-padded seconds", () => {
    expect(formatDuration(65_000)).toBe("1m05s")
    expect(formatDuration(3_602_000)).toBe("60m02s")
  })
})

describe("formatDurationLabel (秒数档位文案)", () => {
  // formatDurationLabel 的 t 替身：把键与插值 n 拼进返回值，断言键选择与
  // 变量换算。Cast 到 TFunction（branded 类型）与生产签名对齐（error.test
  // 同款做法）。
  const t = ((key: string, opts?: Record<string, unknown>) =>
    opts && "n" in opts ? `${key}:${String(opts.n)}` : key) as TFunction

  it("0 → zeroKey（autoTuck 的「关闭」）；未传 zeroKey 按 0 秒渲染", () => {
    expect(formatDurationLabel(0, t, { zeroKey: "x.off" })).toBe("x.off")
    expect(formatDurationLabel(0, t)).toBe("common.seconds:0")
  })

  it("<60 秒 → common.seconds；<1 小时 → common.minutes（除以 60）", () => {
    expect(formatDurationLabel(5, t)).toBe("common.seconds:5")
    expect(formatDurationLabel(59, t)).toBe("common.seconds:59")
    expect(formatDurationLabel(60, t)).toBe("common.minutes:1")
    expect(formatDurationLabel(300, t)).toBe("common.minutes:5")
  })

  it("≥1 小时 → common.hours（除以 3600）——预设表加小时档不用改分档", () => {
    expect(formatDurationLabel(3600, t)).toBe("common.hours:1")
    expect(formatDurationLabel(7200, t)).toBe("common.hours:2")
  })
})

describe("spanParts (时长拆分)", () => {
  const cases: Array<{
    name: string
    ms: number | null | undefined
    want: unknown
  }> = [
    {
      name: "under a minute rounds down to 0 minutes",
      ms: 59_999,
      want: { days: 0, hours: 0, minutes: 0 },
    },
    {
      name: "a few minutes",
      ms: 5 * 60_000 + 30_000,
      want: { days: 0, hours: 0, minutes: 5 },
    },
    {
      name: "hours and minutes",
      ms: 2 * 3_600_000 + 5 * 60_000,
      want: { days: 0, hours: 2, minutes: 5 },
    },
    {
      name: "days and hours",
      ms: 3 * 86_400_000 + 7 * 3_600_000,
      want: { days: 3, hours: 7, minutes: 0 },
    },
    { name: "null is null (no duration)", ms: null, want: null },
    { name: "zero is null", ms: 0, want: null },
    { name: "negative is null (times crossed)", ms: -1000, want: null },
    { name: "NaN is null", ms: NaN, want: null },
  ]
  for (const c of cases) {
    it(c.name, () => {
      expect(spanParts(c.ms)).toEqual(c.want)
    })
  }
})

describe("spanLabelKey (时长文案键选择)", () => {
  it("days win → days+hours label", () => {
    expect(spanLabelKey({ days: 3, hours: 7, minutes: 0 })).toEqual({
      key: "span.daysHours",
      vars: { d: 3, h: 7 },
    })
  })
  it("hours + minutes → hoursMinutes; hours only → hours", () => {
    expect(spanLabelKey({ days: 0, hours: 2, minutes: 5 })).toEqual({
      key: "span.hoursMinutes",
      vars: { h: 2, m: 5 },
    })
    expect(spanLabelKey({ days: 0, hours: 2, minutes: 0 })).toEqual({
      key: "span.hours",
      vars: { h: 2 },
    })
  })
  it("minutes only → minutes label; null → null (caller renders the dash)", () => {
    expect(spanLabelKey({ days: 0, hours: 0, minutes: 5 })).toEqual({
      key: "span.minutes",
      vars: { m: 5 },
    })
    expect(spanLabelKey(null)).toBeNull()
  })
})

describe("spanMsOf (有效时长谓词)", () => {
  it("last_active − started 的毫秒差", () => {
    expect(
      spanMsOf({
        started_at: "2026-08-01T10:00:00Z",
        last_active_at: "2026-08-01T11:30:00Z",
      }),
    ).toBe(90 * 60_000)
  })
  it("空串（时间缺采）→ null——判空是谓词的一部分，不靠 NaN 碰巧", () => {
    expect(
      spanMsOf({ started_at: "", last_active_at: "2026-08-01T11:30:00Z" }),
    ).toBeNull()
    expect(
      spanMsOf({ started_at: "2026-08-01T10:00:00Z", last_active_at: "" }),
    ).toBeNull()
  })
  it("不可解析 / 时间交叉 → null（时长桶跳过，不数垃圾）", () => {
    expect(
      spanMsOf({
        started_at: "not-a-time",
        last_active_at: "2026-08-01T11:30:00Z",
      }),
    ).toBeNull()
    expect(
      spanMsOf({
        started_at: "2026-08-01T11:30:00Z",
        last_active_at: "2026-08-01T10:00:00Z",
      }),
    ).toBeNull()
  })
})

describe("formatSize", () => {
  it("em-dash for nullish / non-positive / non-finite", () => {
    expect(formatSize(null)).toBe("—")
    expect(formatSize(undefined)).toBe("—")
    expect(formatSize(0)).toBe("—")
    expect(formatSize(-5)).toBe("—")
    expect(formatSize(Number.NaN)).toBe("—")
  })

  it("bytes under 1 KiB → plain B", () => {
    expect(formatSize(512)).toBe("512 B")
    expect(formatSize(1023)).toBe("1023 B")
  })

  it("scales to KB / MB / GB at the right thresholds", () => {
    expect(formatSize(2048)).toBe("2.0 KB")
    expect(formatSize(1024 * 1024 * 1.5)).toBe("1.5 MB")
    expect(formatSize(1024 ** 3 * 2)).toBe("2.00 GB")
  })
})

describe("formatTime / formatTimeExact", () => {
  it("formatTime：当年 MM/DD HH:mm；非当年补年份前缀（跨年防误读）", () => {
    const now = dayjs()
    expect(formatTime(now.toISOString())).toBe(now.format("MM/DD HH:mm"))
    const otherYear =
      now.year() > 2020
        ? dayjs("2020-06-15T08:05:00")
        : dayjs("2035-06-15T08:05:00")
    expect(formatTime(otherYear.toISOString())).toMatch(
      /^\d{4}\/\d{2}\/\d{2} \d{2}:\d{2}$/,
    )
  })

  it("formatTimeExact：恒 YYYY-MM-DD HH:mm——相对时间悬浮的绝对时刻必须带年份", () => {
    expect(formatTimeExact("2026-08-27T09:30:00")).toBe("2026-08-27 09:30")
    expect(formatTimeExact(1_756_267_800_000)).toMatch(
      /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/,
    )
  })

  it("空值 → —；坏输入回落原值（两函数同一对空值规则）", () => {
    expect(formatTime(null)).toBe("—")
    expect(formatTimeExact(null)).toBe("—")
    expect(formatTime("")).toBe("—")
    expect(formatTimeExact("")).toBe("—")
    expect(formatTime("not-a-time")).toBe("not-a-time")
    expect(formatTimeExact("not-a-time")).toBe("not-a-time")
  })
})

describe("formatRelative (相对时间出口)", () => {
  it("空值 → —（与 formatTime 同一对空值规则）", () => {
    expect(formatRelative(null)).toBe("—")
    expect(formatRelative(undefined)).toBe("—")
    expect(formatRelative("")).toBe("—")
    expect(formatRelative(0)).toBe("—")
  })

  it("相对措辞走 fromNow，语言随 dayjs locale（插件注册收口 @/i18n/languages，本文件 import 即与生产同路径）", () => {
    const ts = dayjs().subtract(3, "hour").valueOf()
    expect(formatRelative(ts)).toBe("3 hours ago")
    expect(formatRelative(ts)).toBe(dayjs(ts).fromNow())
  })
})

describe("formatDay", () => {
  it("renders an ISO day as MM/DD", () => {
    expect(formatDay("2026-07-28")).toBe("07/28")
  })

  it("null / invalid → placeholder or raw", () => {
    expect(formatDay(null)).toBe("—")
    expect(formatDay("not-a-day")).toBe("not-a-day")
  })
})

describe("dateInputToDay", () => {
  it("trims a date input to the day, or null when blank", () => {
    expect(dateInputToDay("2026-07-28")).toBe("2026-07-28")
    expect(dateInputToDay("  2026-07-28  ")).toBe("2026-07-28")
    expect(dateInputToDay("")).toBeNull()
    expect(dateInputToDay("   ")).toBeNull()
  })
})
