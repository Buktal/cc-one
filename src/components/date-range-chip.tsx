// DateRangeChip — 共享时间范围 popover: 一排预设按钮 (今天 / 7天 / 30天 /
// 全部) + 日历弹层选范围. 纯展示: 调用方拥有值 (preset / fromDay / toDay) 与
// 回调 (onPreset / onFromDay / onToDay)。唯一调用方是共享 FilterBar
// (@/components/filter-bar, 值经 useDateRangeFilter 做 Redux filterSlice 接
// 线) —— 看板 / 日志 / 会话工作台的日期 chip 由此同源. 预设按钮清单与 chip
// 标签回退 key 也在本组件 (usage.control.* 命名空间, 两处工具栏的三语文案逐
// 字相同——此前 sessions 与 usage 各抄一份预设表, 收进本组件单一归属).
//
// `onPreset` 只回传 preset 值本身; 由调用方负责同时落具体 day 边界
// (presetPatch) —— 本组件不碰任何状态, 只上报点击.
//
// 日历 (shadcn Calendar / react-day-picker range mode): 选中日期即回调
// onFromDay / onToDay (与旧原生 date input 的 onChange 同语义), 调用方转入
// custom 预设。日历上的选中态 = 当前 EFFECTIVE 窗口 (动态预设也高亮当天/
// 近 7 天), 点击任何日期即切 custom。range 模式重选时旧 to 会被清掉 (半选
// 态), 与「选范围」的心智一致。

import type { Locale } from "date-fns/locale"
import { enUS, ja, zhCN } from "date-fns/locale"
import dayjs from "dayjs"
import { CalendarRange } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Calendar } from "@/components/ui/calendar"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import { effectiveDays, type Preset } from "@/lib/date-range"
import { cn } from "@/lib/utils"

/** 显示语言 → date-fns locale (日历月名 / 星期名跟随界面语言)。与 dayjs
 *  locale 映射并列, 见 src/i18n/languages.ts。 */
const DATE_FNS_LOCALES: Record<string, Locale> = {
  en: enUS,
  zh: zhCN,
  ja,
}

/** 可选预设 —— "custom" 由日历选日期隐式触发, 永不作为按钮出现. */
type SelectablePreset = Exclude<Preset, "custom">

/** 一个预设按钮: 值 + 其 i18n key. */
interface DateRangePreset {
  value: SelectablePreset
  key: string
}

/** 预设按钮清单（单一归属——usage 与 sessions 工具栏共用同一份, 三语文案
 *  逐字相同）. */
const PRESETS: DateRangePreset[] = [
  { value: "today", key: "usage.control.today" },
  { value: "7d", key: "usage.control.last7d" },
  { value: "30d", key: "usage.control.last30d" },
  { value: "all", key: "usage.control.all" },
]

/** chip 标签在 preset === "all" 时的文案 key（「全部时间」）. */
const ALL_TIME_KEY = "usage.control.allTime"

/** 当前 preset 在 PRESETS 里找不到匹配项时回退的 chip 标签 key. */
const DATE_RANGE_KEY = "usage.control.dateRange"

export interface DateRangeChipProps {
  preset: Preset
  fromDay: string
  toDay: string
  onPreset: (p: Preset) => void
  onFromDay: (d: string) => void
  onToDay: (d: string) => void
  /** popover 相对触发器的对齐. */
  align?: "start" | "end"
}

export function DateRangeChip({
  preset,
  fromDay,
  toDay,
  onPreset,
  onFromDay,
  onToDay,
  align = "start",
}: DateRangeChipProps) {
  const { t, i18n } = useTranslation()
  // 日期框显示 EFFECTIVE 天 —— 动态预设 (如昨天点的「今天」) 渲染当天, 而非
  // 冻结的存储值.
  const { from_day: effFrom, to_day: effTo } = effectiveDays({
    range_preset: preset,
    from_day: fromDay,
    to_day: toDay,
  })
  const label =
    preset === "all"
      ? t(ALL_TIME_KEY)
      : preset !== "custom"
        ? t(PRESETS.find((p) => p.value === preset)?.key ?? DATE_RANGE_KEY)
        : fromDay || toDay
          ? fromDay === toDay
            ? fromDay || "…"
            : `${fromDay || "…"} → ${toDay || "…"}`
          : t(ALL_TIME_KEY)

  return (
    <Popover>
      <PopoverTrigger
        render={
          <button
            type="button"
            className="border-border bg-card hover:bg-hover flex h-8 max-w-full min-w-0 items-center gap-1.5 rounded-md border px-3 text-sm whitespace-nowrap"
          >
            <CalendarRange className="text-muted-foreground size-3.5 shrink-0" />
            <span className="min-w-0 truncate">{label}</span>
          </button>
        }
      />
      <PopoverContent align={align} className="w-auto p-3">
        <div className="bg-muted mb-1 inline-flex items-center gap-0.5 rounded-md p-0.5">
          {PRESETS.map((p) => (
            <button
              key={p.value}
              type="button"
              onClick={() => onPreset(p.value)}
              className={cn(
                "focus-visible:ring-ring/40 rounded-[5px] px-2.5 py-1 text-xs font-medium transition-colors outline-none focus-visible:ring-2",
                preset === p.value
                  ? "bg-accent-tint text-accent-brand-strong"
                  : "text-muted-foreground hover:bg-hover hover:text-foreground",
              )}
            >
              {t(p.key)}
            </button>
          ))}
        </div>
        {/* 日历 range 模式: 选中日期即回调, 与旧原生 date input 的
            onChange 同语义 (调用方转 custom 预设)。日历选中态显示当前
            EFFECTIVE 窗口, 动态预设 (今天 / 7 天…) 也如实高亮。 */}
        <Calendar
          mode="range"
          locale={DATE_FNS_LOCALES[i18n.language] ?? enUS}
          selected={
            effFrom || effTo
              ? {
                  from: effFrom ? dayjs(effFrom).toDate() : undefined,
                  to: effTo ? dayjs(effTo).toDate() : undefined,
                }
              : undefined
          }
          onSelect={(range) => {
            // range 模式: 点第一下只有 from (旧 to 清空成半选态), 点第二下
            // 补全 to。取消选择 (range undefined) 不动作。
            if (!range?.from) return
            onFromDay(dayjs(range.from).format("YYYY-MM-DD"))
            onToDay(range.to ? dayjs(range.to).format("YYYY-MM-DD") : "")
          }}
        />
      </PopoverContent>
    </Popover>
  )
}
