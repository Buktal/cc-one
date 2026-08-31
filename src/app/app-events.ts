// Tauri 事件名单（TS 侧消费源）。
//
// 事件名是 Rust ↔ TS 之间的隐式契约：emit 只发生在 Rust 侧的
// src-tauri/src/events.rs（常量 + 类型化 emit helper），本文件是同一份名字的
// TS 镜像——**改这里的名字必须同改那边**；名字对不上即静默失联（监听不到、
// 无任何报错）。事件名字符串统一 snake_case，两侧一致。
//
// 所有 listen 都必须经 listenAppEvent：事件名只能取自本表的常量，散写的裸
// 字符串类型不过（AppEventName 之外的值编译报错）。

import { listen, type UnlistenFn } from "@tauri-apps/api/event"

/** Store 整体写（采集 / 同步）后的失效信号：失效整个 Store 聚合 tag。 */
export const USAGE_CHANGED = "usage_changed" as const

/** 会话域写（收藏 / 自定义标题 / 分组归属 / 分组 CRUD）后的失效信号。 */
export const SESSIONS_CHANGED = "sessions_changed" as const

/** 供应商域写（CRUD / 重排 / 切换 / live / CC-Switch 导入）后的失效信号。 */
export const PROVIDERS_CHANGED = "providers_changed" as const

/** 托盘点开主窗口 → 前端退出 lightweight 模式，展示完整 dashboard。 */
export const TRAY_SHOW_MAIN = "tray_show_main" as const

/** 主窗口关闭被拦（close_behavior = Ask）→ 弹「最小化 / 退出」对话框。 */
export const CLOSE_REQUESTED = "close_requested" as const

/** 本表全部事件名——listenAppEvent 只接受这些。 */
export type AppEventName =
  | typeof USAGE_CHANGED
  | typeof SESSIONS_CHANGED
  | typeof PROVIDERS_CHANGED
  | typeof TRAY_SHOW_MAIN
  | typeof CLOSE_REQUESTED

/**
 * listen 的唯一入口：事件名限定在名单内。所有事件的 payload 都是 Rust 侧的
 * `()`，handler 无参。
 */
export function listenAppEvent(
  event: AppEventName,
  handler: () => void,
): Promise<UnlistenFn> {
  return listen(event, handler)
}
