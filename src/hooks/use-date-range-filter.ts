// 时间范围筛选的 Redux filterSlice 适配：DateRangeChip 的受控值 + 写回调。
// usage 的 ControlCard / ControlBar 与 sessions 工具栏共用同一份「动态预设不
// 存日期、日历选日期转 custom」的写语义（ADR-0008）——此前两处各写一份
// dispatch 链，收敛到这一个 hook；补丁形状本身在 filterSlice 的 presetPatch /
// dayPatch（可测），本 hook 只做 Redux 接线。

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
