// Calendar heatmap — the contribution graph (#119 指标①). 按天一格的产出
// 节律（GitHub 贡献图式周历：列 = 周、行 = 周一..周日），跨度完全随全局
// 筛选窗口伸缩（「近一年」≈ 53 列满幅；「今天」= 1 格；「全部」= 数据覆盖
// 的整段历史——窄窗自然退化成数格，宽窗横向滚动，图形自带统计窗口是禁区）。
// 色阶 = 非零日四分位（NONE + 四档，calendarCells 口径；绝对值线性会被重度
// 使用者的量级压成两档可读，分位档任何量级都保持五档可辨）。中性墨阶
// --heat-*（mode-scoped，皮肤不写中性），不借四桶色以免 token 语义串味。
// 点击一格 = 窗口收窄到该日（dayRangePatch，from = to = 该日）——与
// DistRow「点 = 收窄全局筛选」同一交互词汇。格数据与趋势图同一条
// useTrendQuery Day 缓存（一 filterId 一份数据多图消费）。

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
import { calendarCells } from "@/features/usage/derive-rhythm"
import { formatCost, formatCount, formatTokens } from "@/lib/format"
import { cn } from "@/lib/utils"

// 格尺寸（图标级定尺例外）与列距：热力格是「本身就该定大小」的网格原子，
// 容器横向滚动自适应，格子不随卡宽拉伸。
const CELL = 11
const GAP = 2
const PITCH = CELL + GAP

export function CalendarHeatmap({
  filter,
  onPickDay,
}: {
  filter: FilterState
  /** 点击一格 → 全局窗口收窄到该日（from = to = 该日）。 */
  onPickDay: (day: string) => void
}) {
  const { t } = useTranslation()
  const {
    data: trend = [],
    isLoading,
    error,
  } = useTrendQuery({ filter, bucket: "Day" })
  const cells = useMemo(() => calendarCells(trend), [trend])
  // 筛选已是「单日 custom」→ 高亮该格：点击收窄后的反馈态（再点同格无
  // 取消语义——取消走时间 chip 的预设按钮，与日期筛选的既有操作一致）。
  const selectedDay =
    filter.range_preset === "custom" &&
    filter.from_day !== "" &&
    filter.from_day === filter.to_day
      ? filter.from_day
      : undefined
  const totalTokens = cells.reduce((s, c) => s + c.tokens, 0)
  const activeDays = cells.filter((c) => c.tokens > 0).length
  // 首格的行 = 首列下移量（周一..周日的空格数），网格先补隐形占位再流式
  // 排格子，列/行几何与 derive-rhythm 的 col/row 完全一致。占位键 = 各空位
  // 真实日期的 epoch（它们就是窗口首日之前的那些格子位，稳定且非索引）。
  const lead = cells.length > 0 ? cells[0].row : 0
  const leadKeys =
    cells.length > 0
      ? Array.from({ length: lead }, (_, i) =>
          dayjs(cells[0].day)
            .subtract(lead - i, "day")
            .valueOf(),
        )
      : []
  // 月标：列首换月时贴列打标（比较用 YYYY-MM，显示用本地语月名）。
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
  // 周几行标（周一/周三/周五/周日四档）——星期名归 dayjs locale，与界面
  // 语言同步（DateRangeChip 的日历同源），不另设一份 i18n 键。行 0 = 周一：
  // dayjs 的 .day(n) 以周日为 0，故取 n = row + 1。
  const weekdayLabel = (row: number) =>
    dayjs()
      .day(row + 1)
      .format("dd")

  return (
    <Card interactive>
      <CardHeader>
        <CardTitle>{t("usage.calendar.title")}</CardTitle>
        <CardAction>
          <div className="text-muted-foreground flex items-center gap-1 text-[11px]">
            <span>{t("usage.calendar.less")}</span>
            {[0, 1, 2, 3, 4].map((l) => (
              <span
                key={l}
                className="size-[11px] shrink-0 rounded-[2px]"
                style={{ backgroundColor: `var(--heat-${l})` }}
              />
            ))}
            <span>{t("usage.calendar.more")}</span>
          </div>
        </CardAction>
      </CardHeader>
      <CardContent>
        <QueryState
          isLoading={isLoading}
          error={error}
          isEmpty={cells.length === 0}
          emptyLabel={t("usage.calendar.empty")}
          emptyDescription={t("usage.calendar.emptyDesc")}
        >
          {/* 窗口聚合句 —— GitHub「N contributions」同款的整窗读数。 */}
          <div className="text-muted-foreground pb-2 text-xs tabular-nums">
            {t("usage.calendar.summary", {
              tokens: formatTokens(totalTokens),
              days: activeDays,
            })}
          </div>
          <div className="flex gap-1.5">
            <div
              aria-hidden="true"
              className="grid shrink-0 text-[10px]"
              style={{
                gridTemplateRows: `repeat(7, ${CELL}px)`,
                rowGap: GAP,
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
            <div className="min-w-0 flex-1 overflow-x-auto pb-1">
              <div
                className="relative"
                style={{ height: 14, marginBottom: GAP }}
              >
                {monthLabels.map((m) => (
                  <span
                    key={`${m.col}-${m.label}`}
                    className="text-muted-foreground absolute top-0 text-[10px] leading-[14px] whitespace-nowrap"
                    style={{ left: m.col * PITCH }}
                  >
                    {m.label}
                  </span>
                ))}
              </div>
              <div
                className="grid grid-flow-col"
                style={{
                  gridTemplateRows: `repeat(7, ${CELL}px)`,
                  gridAutoColumns: `${CELL}px`,
                  gap: GAP,
                }}
              >
                {leadKeys.map((v) => (
                  <span key={v} aria-hidden="true" />
                ))}
                {cells.map((c) => {
                  const selected = c.day === selectedDay
                  return (
                    <button
                      key={c.day}
                      type="button"
                      // 原生 title：365 格的 hover 读数不逐格挂 React 浮层
                      // （数据大了不卡）；格内精确值靠它 + 点击收窄复看。
                      title={[
                        c.day,
                        formatTokens(c.tokens),
                        `${t("usage.hero.requests")} ${formatCount(c.requests)}`,
                        `${t("usage.caliber.priceEstimate")} ${formatCost(c.cost)}`,
                      ].join(" · ")}
                      aria-label={t("usage.calendar.pickDay", { day: c.day })}
                      aria-pressed={selected}
                      onClick={() => onPickDay(c.day)}
                      className={cn(
                        "rounded-[2px] outline-none focus-visible:ring-2 focus-visible:ring-ring/40",
                        selected
                          ? "ring-1 ring-ring"
                          : "hover:ring-1 hover:ring-ring/50",
                      )}
                      style={{ backgroundColor: `var(--heat-${c.level})` }}
                    />
                  )
                })}
              </div>
            </div>
          </div>
        </QueryState>
      </CardContent>
    </Card>
  )
}
