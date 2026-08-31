// Display formatting helpers. The JS layer never computes
// cost — these are display-only shapers for numbers, currency, dates.
//
// Locale policy: token counts are ALWAYS K/M/B (international-neutral,
// language-independent); cost is always USD `$`; dates are always the compact
// numeric `MM/DD HH:mm`. Only the relative-time words (`fromNow`) follow the
// language — driven by the dayjs locale set in `@/i18n/languages`. So nothing
// here hard-codes a dayjs locale.
//
// Metric DSL (统一指标展示): a metric segment is `标签 数量` plus an optional
// ` · 占比` share (`formatMetricSeg`); segments join with ` · `
// (`formatMetricLine`). Values: tokens/counts compact to K/M/B, shares and
// rates are ALWAYS one decimal (no trailing-zero trimming — `96.0%`, not
// `96%`), metric costs are ALWAYS `$` with two decimals. Ledger/pricing
// surfaces that need sub-cent precision use `formatCostPrecise` /
// `formatCostAmount` instead of the metric shapers.

import dayjs from "dayjs"
import type { TFunction } from "i18next"

/** Compact a token count to K/M/B: `3.61M`, `1.2B`, `856`. Language-independent. */
export function formatTokens(n: number | null | undefined): string {
  const v = Number(n ?? 0)
  if (!Number.isFinite(v)) return "0"
  if (v >= 1e9) return `${trim(v / 1e9)}B`
  if (v >= 1e6) return `${trim(v / 1e6)}M`
  if (v >= 1e3) return `${trim(v / 1e3)}K`
  return v.toLocaleString("en-US")
}

/** Count metric (requests / sessions / messages — the DSL's 计数类): K/M/B
 *  like tokens but only from 10K up (`24,670` → `24.7K`, `10,000` → `10K`);
 *  below the threshold the plain grouped integer reads better (`9,999`). UI
 *  chrome counts (paging "共 N 条") are not metrics — those keep formatInt. */
export function formatCount(n: number | null | undefined): string {
  const v = Math.trunc(Number(n ?? 0))
  if (!Number.isFinite(v)) return "0"
  if (v >= 1e9) return `${oneDecimal(v / 1e9)}B`
  if (v >= 1e6) return `${oneDecimal(v / 1e6)}M`
  if (v >= 1e4) return `${oneDecimal(v / 1e3)}K`
  return v.toLocaleString("en-US")
}

/** USD cost with 4 decimals, no currency symbol — `1.7564`. Null/0 →
 *  `0.0000`. Tables that carry the `$` unit in the column header use this so
 *  the symbol doesn't repeat per cell. */
export function formatCostAmount(usd: number | null | undefined): string {
  const v = Number(usd ?? 0)
  if (!Number.isFinite(v)) return "0.0000"
  return v.toFixed(4)
}

/** Metric USD cost — the DSL rule: always `$` with exactly two decimals
 *  (`$12.34`, `$0.00`). KPI values, footer lines and row-level cost segments
 *  all use this; sub-cent precision surfaces (cost breakdowns, pricing
 *  tables) use formatCostPrecise / formatCostAmount instead. */
export function formatCost(usd: number | null | undefined): string {
  const v = Number(usd ?? 0)
  if (!Number.isFinite(v)) return "$0.00"
  return `$${v.toFixed(2)}`
}

/** Precise USD (`$0.0003`) — the `$`-prefixed form of formatCostAmount for
 *  ledger surfaces where per-bucket cents matter (log detail cost lines). */
export function formatCostPrecise(usd: number | null | undefined): string {
  return `$${formatCostAmount(usd)}`
}

/** Integer with thousands separators. */
export function formatInt(n: number | null | undefined): string {
  const v = Math.trunc(Number(n ?? 0))
  return v.toLocaleString("en-US")
}

/** Ratio in [0,1] → percent string. DSL: shares and rates are ALWAYS one
 *  decimal with the trailing zero kept (`90.2%`, `96.0%`, `0.0%`) — column
 *  widths stay stable across values. */
export function formatPct(rate: number | null | undefined): string {
  const v = Number(rate ?? 0)
  if (!Number.isFinite(v)) return "0.0%"
  return `${(v * 100).toFixed(1)}%`
}

/** Plain ratio (e.g. requests per turn) — the DSL's one-decimal rule for the
 *  ratio class, without the percent scaling. Non-finite → `0.0`. */
export function formatRatio(n: number | null | undefined): string {
  const v = Number(n ?? 0)
  if (!Number.isFinite(v)) return "0.0"
  return v.toFixed(1)
}

/** Milliseconds → `12.3s` / `1m05s`. Em-dash when absent / non-positive. */
export function formatDuration(ms: number | null | undefined): string {
  const v = Number(ms ?? 0)
  if (!Number.isFinite(v) || v <= 0) return "—"
  if (v < 60_000) return `${(v / 1000).toFixed(1)}s`
  const m = Math.floor(v / 60_000)
  const sec = Math.round((v % 60_000) / 1000)
  return `${m}m${sec.toString().padStart(2, "0")}s`
}

// --------------------------------------------- 秒数档位文案（间隔/延时类） ----

/** 秒数 → 档位展示文案，「秒/分/时」分档决策的唯一实现（架构审查Ⅵ候选⑧a，
 *  general-card 曾内联 4 份同式三元）：0 → zeroKey（如 autoTuck 的「关闭」；
 *  未传 zeroKey 按 0 秒渲染），<60 秒 → common.seconds，<1 小时 →
 *  common.minutes（除以 60），≥1 小时 → common.hours（除以 3600）。只定键
 *  与插值变量，文案翻译由调用方传入的 t 完成——预设表将来加「小时」档
 *  （如 push 7200）时，选项列表与触发器共用本函数，不可能再各说各话。 */
export function formatDurationLabel(
  secs: number,
  t: TFunction,
  opts?: { zeroKey?: string },
): string {
  if (secs === 0 && opts?.zeroKey) return t(opts.zeroKey)
  if (secs < 60) return t("common.seconds", { n: secs })
  if (secs < 3600) return t("common.minutes", { n: secs / 60 })
  return t("common.hours", { n: secs / 3600 })
}

// ------------------------------------------------------- session span trio ----
// 时长三件套（架构审查Ⅲ候选⑩）：ms → 有效性谓词 → 天/时/分拆分 → i18n 键。
// 与「会话」无关——任何「起止时间对 → 展示时长」的面（会话详情 / 会话统计
// 右栏 / usage KPI 最长会话）共用同一套口径，此前住 features/sessions 被
// usage 跨 feature 借（kpi-band），usage/derive 还手抄过一份无判空的副本。

/** 天/时/分的时长拆分（spanParts 的返回形状）。 */
export interface SpanParts {
  days: number
  hours: number
  minutes: number
}

/** 时长 ms → 天/时/分。null 当 ms 缺失 / 非有限 / 非正——调用方渲染占位符
 *  （—）而不是一个负的伪时长。 */
export function spanParts(ms: number | null | undefined): SpanParts | null {
  const v = Number(ms ?? 0)
  if (!Number.isFinite(v) || v <= 0) return null
  const totalMinutes = Math.floor(v / 60_000)
  return {
    days: Math.floor(totalMinutes / (24 * 60)),
    hours: Math.floor((totalMinutes % (24 * 60)) / 60),
    minutes: totalMinutes % 60,
  }
}

/** 时长的展示文案选择：有天数 → 天+小时；有小时 → 小时+分钟（有分钟时）/
 *  纯小时；否则纯分钟；无时长 → null。只选键与插值变量，文案翻译仍由调用
 *  方 `t()` 完成。null = 无可用时长，调用方渲染占位符（—）。 */
export function spanLabelKey(span: SpanParts | null): {
  key: "span.daysHours" | "span.hoursMinutes" | "span.hours" | "span.minutes"
  vars: Record<string, number>
} | null {
  if (!span) return null
  if (span.days > 0) {
    return {
      key: "span.daysHours",
      vars: { d: span.days, h: span.hours },
    }
  }
  if (span.hours > 0) {
    return span.minutes > 0
      ? {
          key: "span.hoursMinutes",
          vars: { h: span.hours, m: span.minutes },
        }
      : { key: "span.hours", vars: { h: span.hours } }
  }
  return { key: "span.minutes", vars: { m: span.minutes } }
}

/** 「有效时长」谓词 + 换算：起止时间对 → 时长 ms。空串（时间缺采）/ 不可
 *  解析 / 非正一律 null——时长桶与累计时长跳过这些行，不数垃圾。这是带
 *  判空的权威版（usage 侧曾手抄一份无判空副本，靠 NaN 碰巧等价）。 */
export function spanMsOf(row: {
  started_at: string
  last_active_at: string
}): number | null {
  if (!row.started_at || !row.last_active_at) return null
  const ms = dayjs(row.last_active_at).diff(row.started_at)
  return Number.isFinite(ms) && ms > 0 ? ms : null
}

/** Bytes → `1.2 KB` / `3.4 MB` / `5.67 GB`. Em-dash when absent / non-finite. */
export function formatSize(bytes: number | null | undefined): string {
  const v = Number(bytes ?? 0)
  if (!Number.isFinite(v) || v <= 0) return "—"
  if (v < 1024) return `${Math.round(v)} B`
  if (v < 1024 ** 2) return `${(v / 1024).toFixed(1)} KB`
  if (v < 1024 ** 3) return `${(v / 1024 ** 2).toFixed(1)} MB`
  return `${(v / 1024 ** 3).toFixed(2)} GB`
}

/** Timestamp (ISO string or epoch ms) → `MM/DD HH:mm`; 非当年补年份前缀
 *  (`YYYY/MM/DD HH:mm`)——跨年的时间只写 MM/DD 会被读成今年。Falls back
 *  to the raw value on bad input. */
export function formatTime(ts: string | number | null | undefined): string {
  if (!ts) return "—"
  const d = dayjs(ts)
  if (!d.isValid()) return String(ts)
  return d.format(
    d.isSame(dayjs(), "year") ? "MM/DD HH:mm" : "YYYY/MM/DD HH:mm",
  )
}

/** Timestamp → `YYYY-MM-DD HH:mm`（含年份的精确时刻）。相对时间
 *  （`fromNow`）悬浮展示的绝对时间用这版——相对措辞已经丢了精度，悬浮里
 *  年份必须补全，不做跨年特例。空值 → `—`，坏输入回落原值（与 formatTime
 *  同一对空值规则）。 */
export function formatTimeExact(
  ts: string | number | null | undefined,
): string {
  if (!ts) return "—"
  const d = dayjs(ts)
  if (!d.isValid()) return String(ts)
  return d.format("YYYY-MM-DD HH:mm")
}

/** Timestamp → 相对措辞（`fromNow`，语言随 dayjs locale——插件注册与切换
 *  收口在 `@/i18n/languages`，这里不注册）。空值 → 「—」，与 formatTime
 *  同一对空值规则。相对时间文案的纯函数出口（架构审查Ⅵ候选⑧b）：
 *  RelativeTime 组件渲染悬浮触发文本用它，不便嵌 Tooltip 的面（Popover
 *  触发器、metric 段拼接）直接取它——不再散落裸 `dayjs().fromNow()`。 */
export function formatRelative(ts: string | number | null | undefined): string {
  if (!ts) return "—"
  return dayjs(ts).fromNow()
}

/** ISO day `yyyy-mm-dd` → `MM/DD`. */
export function formatDay(day: string | null | undefined): string {
  if (!day) return "—"
  const d = dayjs(day)
  return d.isValid() ? d.format("MM/DD") : day
}

/** Convert a `<input type="date">` value (yyyy-mm-dd) to a filter day or null. */
export function dateInputToDay(v: string): string | null {
  return v && v.trim() !== "" ? v.trim() : null
}

// ------------------------------------------------------------- metric DSL ----

/** The DSL segment: `标签 数量`, plus ` · 占比` when a share is given — e.g.
 *  `输入 96.37M · 96.0%` or the footer `请求 24.7K`. `value` arrives
 *  pre-formatted (formatTokens / formatCount / formatCost / formatPct …);
 *  `share` is a [0,1] ratio the caller derives from its data. */
export function formatMetricSeg(
  label: string,
  value: string,
  share?: number | null,
): string {
  const base = `${label} ${value}`
  return share == null ? base : `${base} · ${formatPct(share)}`
}

/** `数量 · 占比` — the value half of a segment for layouts that render the
 *  label elsewhere (the hero legend's label+dot sit on the row's left; the
 *  distribution row's entity name is the label). */
export function formatSegValue(value: string, share?: number | null): string {
  return share == null ? value : `${value} · ${formatPct(share)}`
}

/** Join metric segments into one line with the DSL separator ` · `. */
export function formatMetricLine(segs: readonly string[]): string {
  return segs.join(" · ")
}

function trim(n: number): string {
  // 2 decimals, drop trailing zeros for compactness.
  return n
    .toFixed(2)
    .replace(/\.?0+$/, "")
    .trim()
}

function oneDecimal(n: number): string {
  // 1 decimal, drop a trailing zero ("24.7K", "10K").
  return n.toFixed(1).replace(/\.0$/, "")
}
