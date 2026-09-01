// 用量热力（原「用量日历」）——按窗口宽度三形态，全部格子 1fr 随卡宽
// 铺满（不再定宽挤在卡角留白）：
//   ≤7 天    小时矩阵（hour-matrix）：行 = 日、列 = 0–23 时——短窗的
//            Day 桶没有「图形」，小时粒度才有节奏可看。
//   8–70 天  月历：行 = 周、列 = 周一..周日（7 列大格内嵌日期号）——
//            30 天档即真实挂历的样子，而不是几列漂在空白里的碎格。
//   >70 天   周历（GitHub 式）：列 = 周、行 = 周一..周日；列数随窗口
//            伸缩，近一年 53 列满幅，≤14 列时格子够宽同样内嵌日期号。
// 色阶 = 非零值四分位（NONE + 四档主色渐进，heat-shared）。月历/周历
// 同吃 Day 桶：先 zeroFillDailyTrend 把窗口铺满（空日占格），格位才对
// 得上真实星期——后端 GROUP BY 略过空日，缺一天后面全部左移错位。
// 「全部」无界窗口的铺满边界退化为数据首日 → 今天。点击一格 = 窗口收窄
// 到该日（dayRangePatch，from = to = 该日）。跨度完全随全局筛选窗口伸缩
// （图形自带统计窗口是禁区）。

import dayjs from "dayjs"
import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { useTrendQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  type CalendarCell,
  calendarCells,
} from "@/features/usage/derive-calendar"
import { zeroFillDailyTrend } from "@/features/usage/derive-trend"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import { formatDay } from "@/lib/format"
import { HeatCell, HeatLegend, HeatSummary } from "./heat-shared"
import { HourMatrix } from "./hour-matrix"

/** 短窗阈值：窗口 ≤ 7 天时切小时粒度（行 ≤ 7 × 24 列）。 */
const CALENDAR_HOUR_MAX_DAYS = 7

/** 月历档上限：≤ 70 天（10 周）行 = 周、7 列挂历格；再宽转 GitHub 周历。 */
const WEEKGRID_MAX_DAYS = 70

const MONTH_ROW_PX = 40
const WEEK_ROW_PX = 24

/** 筛选已是「单日 custom」→ 高亮该格：点击收窄后的反馈态（再点同格无
 *  取消语义——取消走时间 chip 的预设按钮，与日期筛选的既有操作一致）。 */
function selectedDayOf(filter: FilterState): string | undefined {
  return filter.range_preset === "custom" &&
    filter.from_day !== "" &&
    filter.from_day === filter.to_day
    ? filter.from_day
    : undefined
}

/** Day 桶 → 对齐真实星期的日历格（月历/周历共用）：窗口先铺满再排格，
 *  「全部」无界窗口的边界退化为数据首日 → 今天。查询态一并返回（同一条
 *  useTrendQuery 缓存，两形态不各查一遍）。 */
function useCalendarCells(filter: FilterState): {
  cells: CalendarCell[]
  isLoading: boolean
  error: unknown
} {
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  const {
    data: trend = [],
    isLoading,
    error,
  } = useTrendQuery({
    filter,
    bucket: "Day",
  })
  const cells = useMemo(() => {
    if (trend.length === 0) return []
    const from = fromTs ? dayjs(fromTs) : dayjs(trend[0].day).startOf("day")
    const to = toTs ? dayjs(toTs) : dayjs().endOf("day")
    return calendarCells(zeroFillDailyTrend(trend, from, to, dayjs()))
  }, [trend, fromTs, toTs])
  return { cells, isLoading, error }
}

/** 行/列标的星期名（周一..周日）：dayjs 的 .day(n) 以周日为 0，取 n = 行 + 1。 */
function weekdayLabel(row: number): string {
  return dayjs()
    .day(row + 1)
    .format("dd")
}

export function CalendarHeatmap({
  filter,
  onPickDay,
  spanDays,
}: {
  filter: FilterState
  /** 点击一格 → 全局窗口收窄到该日（from = to = 该日）。 */
  onPickDay: (day: string) => void
  /** 窗口日历天数（null = 无界窗口，如「全部」）——形态判定由此而来。 */
  spanDays: number | null
}) {
  const { t } = useTranslation()
  const hourly = spanDays != null && spanDays <= CALENDAR_HOUR_MAX_DAYS
  const monthly =
    spanDays != null &&
    spanDays > CALENDAR_HOUR_MAX_DAYS &&
    spanDays <= WEEKGRID_MAX_DAYS
  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.calendar.title")}</CardTitle>
        <CardAction>
          <HeatLegend />
        </CardAction>
      </CardHeader>
      <CardContent>
        {hourly ? (
          <HourMatrix filter={filter} onPickDay={onPickDay} />
        ) : monthly ? (
          <MonthGrid filter={filter} onPickDay={onPickDay} />
        ) : (
          <WeekCalendar filter={filter} onPickDay={onPickDay} />
        )}
      </CardContent>
    </Card>
  )
}

/** 月历形态（8–70 天）：行 = 周、列 = 周一..周日。7 列 1fr 大格随卡宽
 *  铺满，格内嵌日期号；行标 = 该周周一，列标 = 星期名。 */
function MonthGrid({
  filter,
  onPickDay,
}: {
  filter: FilterState
  onPickDay: (day: string) => void
}) {
  const { t } = useTranslation()
  const { cells, isLoading, error } = useCalendarCells(filter)
  const totalTokens = cells.reduce((s, c) => s + c.tokens, 0)
  const activeDays = cells.filter((c) => c.tokens > 0).length
  // 行 = 周（col），行内按星期升序；行标 = 该周周一（首格日期回退 row 天）。
  // 行 key = 该周周一的 ISO 日（日期即稳定 id，不用数组索引）。
  const rows: CalendarCell[][] = []
  for (const c of cells) {
    if (rows[c.col] == null) rows[c.col] = []
    rows[c.col].push(c)
  }
  const mondayOf = (r: CalendarCell[]) =>
    dayjs(r[0].day).subtract(r[0].row, "day").format("YYYY-MM-DD")
  const selectedDay = selectedDayOf(filter)

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={cells.length === 0}
      emptyLabel={t("usage.calendar.empty")}
      emptyDescription={t("usage.calendar.emptyDesc")}
    >
      <HeatSummary tokens={totalTokens} activeDays={activeDays} />
      <div className="flex gap-1.5">
        {/* 行标列：paddingTop = 列标行高（12 + GAP），与网格首行对齐。 */}
        <div
          aria-hidden="true"
          className="flex shrink-0 flex-col text-[10px]"
          style={{ rowGap: 3, paddingTop: 12 + 3 }}
        >
          {rows.map((r) => (
            <span
              key={mondayOf(r)}
              className="text-muted-foreground flex items-center justify-end leading-none"
              style={{ height: MONTH_ROW_PX, width: 40 }}
            >
              {r.length > 0 ? formatDay(mondayOf(r)) : ""}
            </span>
          ))}
        </div>
        <div className="min-w-0 flex-1">
          <div
            aria-hidden="true"
            className="grid"
            style={{
              gridTemplateColumns: "repeat(7, 1fr)",
              gap: 3,
              height: 12,
              marginBottom: 3,
            }}
          >
            {[0, 1, 2, 3, 4, 5, 6].map((row) => (
              <span
                key={row}
                className="text-muted-foreground text-[10px] leading-[12px]"
              >
                {weekdayLabel(row)}
              </span>
            ))}
          </div>
          <div className="flex flex-col" style={{ rowGap: 3 }}>
            {rows.map((r) => (
              <div
                key={mondayOf(r)}
                className="grid"
                style={{
                  gridTemplateColumns: "repeat(7, 1fr)",
                  gap: 3,
                  height: MONTH_ROW_PX,
                }}
              >
                {/* 首周的星期前导空位（窗口首日未到周一）；占位键 = 各
                    空位真实日期（首日之前的那些格子位，稳定且非索引）。 */}
                {r === rows[0]
                  ? Array.from({ length: r[0]?.row ?? 0 }, (_, k) => (
                      <span
                        key={dayjs(r[0].day)
                          .subtract(r[0].row - k, "day")
                          .format("YYYY-MM-DD")}
                        aria-hidden="true"
                      />
                    ))
                  : null}
                {r.map((c) => (
                  <HeatCell
                    key={c.day}
                    cell={c}
                    selected={c.day === selectedDay}
                    onPickDay={onPickDay}
                    showDayMark
                  />
                ))}
              </div>
            ))}
          </div>
        </div>
      </div>
    </QueryState>
  )
}

/** 周历形态（>70 天，GitHub 式）：列 = 周、行 = 周一..周日。列数随窗口
 *  伸缩（1fr 铺满），≤14 列时格子够宽内嵌日期号；月标贴换月列顶。 */
function WeekCalendar({
  filter,
  onPickDay,
}: {
  filter: FilterState
  onPickDay: (day: string) => void
}) {
  const { t } = useTranslation()
  const { cells, isLoading, error } = useCalendarCells(filter)
  const totalTokens = cells.reduce((s, c) => s + c.tokens, 0)
  const activeDays = cells.filter((c) => c.tokens > 0).length
  // 首格的星期 = 首列下移量（周一..周日的空格数），网格先补隐形占位再按
  // 列流排格（gridAutoFlow: column），列/行几何与 derive-calendar 一致。
  const lead = cells.length > 0 ? cells[0].row : 0
  const colCount = cells.reduce((m, c) => Math.max(m, c.col + 1), 0)
  const selectedDay = selectedDayOf(filter)
  const showDayMark = colCount <= 14
  // 月标：换月列贴列打标（比较用 YYYY-MM，显示用本地语月名）；1fr 列宽
  // 下用同构网格放置，窄格的标签向右溢出（下格无标，不互撞）。
  const monthLabels: Array<{ col: number; label: string }> = []
  let lastMonth = ""
  for (const c of cells) {
    if (c.row !== 0) continue
    const month = dayjs(c.day).format("YYYY-MM")
    if (month !== lastMonth) {
      lastMonth = month
      monthLabels.push({ col: c.col, label: dayjs(c.day).format("MMM") })
    }
  }

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={cells.length === 0}
      emptyLabel={t("usage.calendar.empty")}
      emptyDescription={t("usage.calendar.emptyDesc")}
    >
      <HeatSummary tokens={totalTokens} activeDays={activeDays} />
      <div className="flex gap-1.5">
        <div
          aria-hidden="true"
          className="grid shrink-0 text-[10px]"
          style={{
            gridTemplateRows: `repeat(7, ${WEEK_ROW_PX}px)`,
            rowGap: 2,
            paddingTop: 14 + 2,
          }}
        >
          {[0, 1, 2, 3, 4, 5, 6].map((row) => (
            <span
              key={row}
              className="text-muted-foreground flex items-center leading-none"
            >
              {row % 2 === 0 ? weekdayLabel(row) : ""}
            </span>
          ))}
        </div>
        <div className="min-w-0 flex-1">
          <div
            aria-hidden="true"
            className="grid"
            style={{
              gridTemplateColumns: `repeat(${colCount}, 1fr)`,
              gap: 2,
              height: 14,
              marginBottom: 2,
            }}
          >
            {monthLabels.map((m) => (
              <span
                key={`${m.col}-${m.label}`}
                className="text-muted-foreground whitespace-nowrap text-[10px] leading-[14px]"
                style={{ gridColumn: `${m.col + 1} / span 1` }}
              >
                {m.label}
              </span>
            ))}
          </div>
          <div
            className="grid"
            style={{
              gridTemplateColumns: `repeat(${colCount}, 1fr)`,
              gridTemplateRows: `repeat(7, ${WEEK_ROW_PX}px)`,
              gridAutoFlow: "column",
              gap: 2,
            }}
          >
            {cells.length > 0
              ? Array.from({ length: lead }, (_, i) => (
                  <span
                    key={dayjs(cells[0].day)
                      .subtract(lead - i, "day")
                      .valueOf()}
                    aria-hidden="true"
                  />
                ))
              : null}
            {cells.map((c) => (
              <HeatCell
                key={c.day}
                cell={c}
                selected={c.day === selectedDay}
                onPickDay={onPickDay}
                showDayMark={showDayMark}
              />
            ))}
          </div>
        </div>
      </div>
    </QueryState>
  )
}
