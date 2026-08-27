// DeviceScopeControl — 设备维度筛选的单一下拉。
//
// 读 listDevices() (经共享 useDeviceOptions) + 读写 filter.device_scope。
// 设备再多也只是下拉里的一个选项，不会撑爆布局，故统一用 Select。单设备
// (Standalone 仅本机) 无切换意义，整体不渲染。
//
// 同一组件两种形态:
//  - ControlBar (看板 tab 栏 / logs 横排): 无外置 label, 选中「全部」时显
//    全称「全部设备」自带身份 (与库的「全部设备」一致); 选中具体设备显其名。
//  - LightweightCard expanded: compact (11px) 适配小窗。
//
// device_scope 在全局 filter, 故看板 / 日志 / lightweight 的设备筛选一并跟随。
// 哨兵映射 / label 解析等筛选下拉规则收敛在共享 FilterSelect。组件住
// src/components（架构审查Ⅲ候选①）：三个域共享的维度选取面不属于任何一家
// feature，设备身份面（选项来源）在 @/lib/device-labels。

import { useMemo } from "react"
import { useTranslation } from "react-i18next"

import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import { patchFilter } from "@/app/store/slices/filterSlice"
import { FilterSelect } from "@/components/filter-select"
import { useDeviceOptions } from "@/lib/device-labels"
import { cn } from "@/lib/utils"

export function DeviceScopeControl({ compact = false }: { compact?: boolean }) {
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

  return (
    <FilterSelect
      ariaLabel={t("filter.device")}
      allLabel={t("filter.allDevice")}
      options={options}
      value={active}
      onChange={(v) => dispatch(patchFilter({ device_scope: v }))}
      className={cn(
        // 宽度策略（自适应 + 上限）收敛在 FilterSelect 本体；compact 小窗
        // 只补字号。长设备名由 line-clamp-1 截断。
        compact && "text-[11px]",
      )}
      // compact 时下拉字号跟随 trigger (11px)。
      contentClassName={cn(compact && "text-[11px]")}
      align="start"
    />
  )
}
