// 小时矩阵——用量热力的短窗形态（≤7 天，含今天）：行 = 日、列 = 0–23 时。
// 短窗的 Day 桶只有寥寥几格，没有可读的「图形」；小时粒度才有当天的节奏
// 可看。格子宽度 1fr 随卡宽铺满——24 列疏格拉伸成时段条阵（今天 = 一行
// 24 格），不再定宽挤在卡角、右侧大片空白。数据 = Hour 桶 + zeroFillTrend
// 铺满窗口（空小时是显式零点，缺位会把列位挤错）；列标 0/6/12/18 与网格
// 同构放置（1fr 列宽下不能再用绝对定位）。点击一格 = 窗口收窄到该日。

import dayjs from "dayjs"
import { useMemo } from "react"
import { useTranslation } from "react-i18next"
import { useTrendQuery } from "@/app/store/api"
import type { FilterState } from "@/app/store/slices/filterSlice"
import { QueryState } from "@/components/query-state"
import { hourMatrixRows } from "@/features/usage/derive-calendar"
import { zeroFillTrend } from "@/features/usage/derive-trend"
import { dayRangeToTs, effectiveDays } from "@/lib/date-range"
import { formatDay } from "@/lib/format"
import { HeatCell, HeatSummary } from "./heat-shared"

const ROW_PX = 24
const GAP = 3

export function HourMatrix({
  filter,
  onPickDay,
}: {
  filter: FilterState
  /** 点击一格 → 全局窗口收窄到该日（from = to = 该日）。 */
  onPickDay: (day: string) => void
}) {
  const { t } = useTranslation()
  const { from_day, to_day } = effectiveDays(filter)
  const { from_ts: fromTs, to_ts: toTs } = dayRangeToTs(from_day, to_day)
  const {
    data: trend = [],
    isLoading,
    error,
  } = useTrendQuery({ filter, bucket: "Hour" })
  const rows = useMemo(() => {
    if (!fromTs || !toTs) return []
    return hourMatrixRows(
      zeroFillTrend(trend, dayjs(fromTs), dayjs(toTs), dayjs()),
    )
  }, [trend, fromTs, toTs])
  const totalTokens = trend.reduce((s, p) => s + Number(p.total_tokens), 0)
  const activeDays = rows.filter((r) =>
    r.cells.some((c) => c.tokens > 0),
  ).length
  const today = dayjs().format("YYYY-MM-DD")
  // 筛选已是「单日 custom」→ 高亮该行（点击收窄后的反馈态）。
  const selectedDay =
    filter.range_preset === "custom" &&
    filter.from_day !== "" &&
    filter.from_day === filter.to_day
      ? filter.from_day
      : undefined

  return (
    <QueryState
      isLoading={isLoading}
      error={error}
      isEmpty={rows.length === 0}
      emptyLabel={t("usage.calendar.empty")}
      emptyDescription={t("usage.calendar.emptyDesc")}
    >
      <HeatSummary tokens={totalTokens} activeDays={activeDays} />
      <div className="flex gap-1.5">
        {/* 行标列：paddingTop = 列标行高（12 + GAP），与网格首行对齐。 */}
        <div
          aria-hidden="true"
          className="flex shrink-0 flex-col text-[10px]"
          style={{ rowGap: GAP, paddingTop: 12 + GAP }}
        >
          {rows.map((r) => (
            <span
              key={r.day}
              className="text-muted-foreground flex items-center justify-end leading-none"
              style={{ height: ROW_PX, width: 34 }}
            >
              {r.day === today ? t("usage.calendar.today") : formatDay(r.day)}
            </span>
          ))}
        </div>
        <div className="min-w-0 flex-1">
          {/* 列标：与主体网格同构的 24 列 1fr，标只放 0/6/12/18 四列。 */}
          <div
            aria-hidden="true"
            className="grid"
            style={{
              gridTemplateColumns: "repeat(24, 1fr)",
              gap: GAP,
              height: 12,
              marginBottom: GAP,
            }}
          >
            {[0, 6, 12, 18].map((h) => (
              <span
                key={h}
                className="text-muted-foreground text-[10px] leading-[12px]"
                style={{ gridColumnStart: h + 1 }}
              >
                {h}
              </span>
            ))}
          </div>
          <div
            className="grid"
            style={{
              gridTemplateColumns: "repeat(24, 1fr)",
              gridAutoRows: ROW_PX,
              gap: GAP,
            }}
          >
            {rows.flatMap((r) =>
              r.cells.map((c) => (
                <HeatCell
                  key={`${r.day}T${c.hour}`}
                  cell={c}
                  selected={r.day === selectedDay}
                  onPickDay={onPickDay}
                />
              )),
            )}
          </div>
        </div>
      </div>
    </QueryState>
  )
}
