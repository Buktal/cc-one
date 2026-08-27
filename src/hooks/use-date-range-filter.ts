// 时间范围筛选的 Redux filterSlice 适配：DateRangeChip 的受控值 + 写回调。
// 「动态预设不存日期、日历选日期转 custom」的写语义（ADR-0008）的组件级适
// 配全仓只有这一份——唯一消费方是共享 FilterBar（@/components/filter-bar，
// 五维筛选条装配单源），看板 / 日志 / 会话工作台的日期 chip 由此同源；补丁
// 形状本身在 filterSlice 的 presetPatch / dayPatch（可测），本 hook 只做
// Redux 接线。

import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  dayPatch,
  patchFilter,
  presetPatch,
} from "@/app/store/slices/filterSlice"
import type { Preset } from "@/lib/date-range"

/** DateRangeChip 的 slice 适配：受控值（preset / fromDay / toDay）+ 写回调
 *  （onPreset / onFromDay / onToDay）。动态预设只存 preset、不存具体日期。 */
export function useDateRangeFilter() {
  const dispatch = useAppDispatch()
  const filter = useAppSelector((s) => s.filter.filter)
  return {
    preset: filter.range_preset,
    fromDay: filter.from_day,
    toDay: filter.to_day,
    onPreset: (p: Preset) => dispatch(patchFilter(presetPatch(p))),
    onFromDay: (d: string) => dispatch(patchFilter(dayPatch("from_day", d))),
    onToDay: (d: string) => dispatch(patchFilter(dayPatch("to_day", d))),
  }
}
