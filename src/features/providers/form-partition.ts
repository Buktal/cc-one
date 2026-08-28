// 表单分区契约——AppProfile.formPartition 能力的组件接口（app-profiles 头
// 注「组件不再写 app === "xxx" 表达事实」的最后一块：每个应用的字段区组件
// 也进表，sheet 骨架一次查表渲染，不再持有任何应用名）。加第六个应用 =
// 补齐表中一行 + 本契约的一个实现，漏配分区即编译错。
//
// 两个关注点分组，分区组件各取所需（不再是一包 13 键的平铺 props）：
// - form   表单态：名称 / 模板变量 / 自动应用开关 / 分类；
// - models 拉模型管线：共享 runFetchModels 的注入面——错误分桶与候选填充
//   全应用同一份，分区不再自持第二套。
// configText + onChange（守卫写回）是所有分区的公共底座：真相源规则见
// provider-form-sheet 头注（configText 单一真相源，写回经 lib/json 的
// guardedRewrite，半截 JSON 不被吞）。
//
// 本文件只放类型（零运行时依赖）：app-profiles（能力表）与 components/（分
// 区实现）都要引用它，独立成文件让两侧之间不产生 import 环。

import type { ComponentType } from "react"

/** 一个应用的表单分区组件类型（AppProfile.formPartition 的赋值类型）。 */
export type FormPartition = ComponentType<FormPartitionProps>

/** 表单态（跨分区通用形状；分区不用的键留在对象里即可，不必解构）。 */
export interface ProviderFormState {
  name: string
  onNameChange: (value: string) => void
  /** 模板变量的输入值（存于 meta；物化只在保存时——见 sheet 的
   *  onTemplateVarChange 注释）。仅 claude 分区消费。 */
  templateValues: Record<string, string>
  onTemplateVarChange: (name: string, value: string) => void
  /** 模型映射的自动应用开关（编辑任一角色同步全部）。仅 claude 分区消费。 */
  autoSync: boolean
  onAutoSyncChange: (checked: boolean) => void
  /** 供应商分类（claude 分区的认证键显隐规则：官方/云厂商隐藏 key 输入）。 */
  category: string
}

/** 拉模型管线（与 gemini / opencode / claude 共用 sheet 的 runFetchModels，
 *  避免错误处理分叉漂移）。无拉模型入口的应用（codex / grok）不解构即可。 */
export interface ProviderFormModels {
  fetching: boolean
  fetchedModels: string[]
  onFetchModels: () => void
  /** 端点被编辑时清空上次拉到的候选（旧端点的列表不可靠）。 */
  onEndpointEdited: () => void
}

/** 一个应用表单分区的 props：sheet 骨架传下的全部接线。 */
export interface FormPartitionProps {
  configText: string
  /** 守卫写回（sheet 的 guardedWrite 直通）：仅当 settingsConfig JSON 合法
   *  时写，返回是否真的写了（调用方据此决定 toast 等副作用）。 */
  onChange: (update: (prev: string) => string) => boolean
  form: ProviderFormState
  models: ProviderFormModels
}
