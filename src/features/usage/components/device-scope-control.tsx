// DeviceScopeControl — 设备维度筛选的单一下拉。
//
// 读 listDevices() (经共享 useDeviceOptions) + 读写 filter.device_scope。
// 设备再多也只是下拉里的一个选项，不会撑爆布局，故统一用 Select。单设备
// (Standalone 仅本机) 无切换意义，整体不渲染。
//
// 同一组件三种形态:
//  - ControlCard (control-card.tsx): 纵卡, 右栏 Row label 已标「设备」, 值显「全部」。
//  - ControlBar (logs 横排): bar=true, 无外置 label, 选中「全部」时显全称「全部设备」
//    自带身份 (与库的「全部设备」一致); 选中具体设备显其名。
//  - LightweightCard expanded: compact (11px) 适配小窗。
//
// device_scope 在全局 filter, 故看板 / 日志 / lightweight 的设备筛选一并跟随。
// 哨兵映射 / label 解析等筛选下拉规则收敛在共享 FilterSelect。

import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"
import { FilterSelect } from "@/components/filter-select"
import { cn } from "@/lib/utils"

import { useDeviceOptions } from "../use-device-options"

export function DeviceScopeControl({
  compact = false,
  align = "start",
  bar = false,
}: {
  compact?: boolean
  align?: "start" | "end"
  /** 横排 ControlBar: 无外置 label, 选中「全部」时显全称「全部设备」自带身份。 */
  bar?: boolean
}) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const scope = useAppSelector((s) => s.filter.filter.device_scope)
  const deviceOptions = useDeviceOptions()
  const options = useMemo(
    () => deviceOptions.map((o) => ({ value: o.id, label: o.label })),
    [deviceOptions],
  )

  // 单设备 (Standalone 仅本机一台): 无切换意义，不渲染。
  if (options.length === 0) return null

  // 选中设备从列表消失 (如对端重置) → 回退「全部」，无设备项高亮。
  const active = options.some((o) => o.value === scope) ? scope : ""
  const allLabel = bar ? t("usage.control.allDevice") : t("usage.control.all")

  return (
    <FilterSelect
      ariaLabel={t("usage.deviceScope.label")}
      allLabel={allLabel}
      options={options}
      value={active}
      onChange={(v) => dispatch(patchFilter({ device_scope: v }))}
      className={cn(
        // 纵卡 ControlCard 内与应用/模型下拉统一 w-36（三下拉等宽对齐）；
        // 横排 ControlBar 与来源下拉同宽 w-30。长设备名由 line-clamp-1
        // 截断。compact 小窗字号更小更没问题。
        "border-border bg-card hover:bg-hover h-8 w-36 rounded-md",
        bar && !compact && "w-30",
        compact && "text-[11px]",
      )}
      // compact 时下拉字号跟随 trigger (11px), 否则用默认。align=end 让右栏
      // 卡片的菜单向左生长, 不溢出视口。
      contentClassName={cn(compact && "text-[11px]")}
      align={align}
    />
  )
}
