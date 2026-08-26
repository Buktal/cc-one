// 设备身份面（架构审查候选⑥）——「一个设备 id 如何变成展示名」与「设备 UI
// 何以存在」的唯一归属。曾住 features/usage 内部路径而四个域反向借用，
// sessions 还手抄了一份 option 构建且已漂移（is_self 有无）；搬到 lib 这个
// 中性归属地后 usage / sessions / library / shell 全部同向依赖。
//
// 两个读取深度，各有语义：
//  - useDeviceLabels()：全量 id → 展示名 Map——详情卡等「必须叫得出名字」的
//    场景，单机时也显示「本设备」而非 id 片段。
//  - useDeviceOptions()：带显隐策略的选项表——≤1 台返回 []，单机部署不渲染
//    设备 UI；表格列是否存在的判断也读这个长度。
//
// vitest（node 环境）可 import：useTranslation / useDevicesQuery 均在 hook 体
// 内获取，模块顶层无外部资源句柄。

import { useTranslation } from "react-i18next"

import { useDevicesQuery } from "@/app/store/api"

export interface DeviceOption {
  id: string
  label: string
  is_self: boolean
}

/**
 * Device display label — THE single derivation (#72: previously the same
 * ternary was copied at four call sites and could drift). This device →
 * localized "This device"; a peer → its display name, falling back to
 * "Unnamed". Callers keep their own policy for whether to render anything at
 * all in the single-device case.
 */
export function deviceOptionLabel(
  d: { is_self: boolean; display_name: string },
  t: (key: string) => string,
): string {
  return d.is_self
    ? t("devices.thisDevice")
    : d.display_name || t("common.unnamed")
}

/** id → 展示名：Map 未命中回退到裸 id 前 8 位。截断策略唯一定义——此前
 *  sessions-view（×2）、recent-requests、request-log-table 各拼一遍同一式。 */
export function deviceLabelOf(
  labels: ReadonlyMap<string, string>,
  id: string,
): string {
  return labels.get(id) ?? id.slice(0, 8)
}

/**
 * 全量 id → 展示名（含单机）——sessions 详情卡的标签源（其手抄 deviceLabel
 * map 与本函数同构）。
 */
export function useDeviceLabels(): Map<string, string> {
  const { t } = useTranslation()
  const { data: devices = [] } = useDevicesQuery()
  const m = new Map<string, string>()
  for (const d of devices) m.set(d.device_id, deviceOptionLabel(d, t))
  return m
}

/**
 * Devices as picker options (labels via [`deviceOptionLabel`]). Returns `[]`
 * when there is ≤1 device, so a single-machine Standalone setup renders no
 * device UI at all.
 */
export function useDeviceOptions(): DeviceOption[] {
  const { t } = useTranslation()
  const { data: devices = [] } = useDevicesQuery()
  if (devices.length <= 1) return []
  return devices.map((d) => ({
    id: d.device_id,
    label: deviceOptionLabel(d, t),
    is_self: d.is_self,
  }))
}

/** id → label lookup for tables / lists. Empty when single-device (no noise).
 *  只在「确有选择余地」时才展示标签的面用这份；恒需命名的面走
 *  [`useDeviceLabels`]。 */
export function useDeviceLabelMap(): Map<string, string> {
  const options = useDeviceOptions()
  return new Map(options.map((o) => [o.id, o.label]))
}
