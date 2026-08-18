// 会话跳转通道的两端 hooks——跳转入口（任意域）与跳转落地（SessionsView）。
// 状态本体在 app/store/slices/sessionJumpSlice（一次性 target）；本文件只做
// 接线：入口写入 target 并切换视图，落地按 target 取回会话行后经调用方的
// setPreview 打开（与列表行点击同一条通道），随后清除 target。

import { useEffect } from "react"

import { useGetSessionQuery } from "@/app/store/api"
import { useAppDispatch, useAppSelector } from "@/app/store/hooks"
import {
  clearSessionJump,
  setSessionJumpTarget,
} from "@/app/store/slices/sessionJumpSlice"
import { setView } from "@/app/store/slices/viewSlice"
import type { SessionRow } from "@/types/generated/bindings"

/** 跳转入口：返回 `(id, deviceId) => void`。先写 target 再切视图——若已在
 *  会话视图，consumer 直接换开目标会话；若不在，SessionsView 挂载后消费。 */
export function useSessionJump(): (id: string, deviceId: string) => void {
  const dispatch = useAppDispatch()
  return (id: string, deviceId: string) => {
    dispatch(setSessionJumpTarget({ id, device_id: deviceId }))
    dispatch(setView("sessions"))
  }
}

/** 跳转落地：target 存在时经 getSession 取回会话行（点击前跳转方通常已把
 *  同一行查进缓存，落地零等待），到达即 onOpen 打开并清除 target；会话不
 *  存在（null，历史用量无会话行）时静默清除，会话列表保持原状。target 与
 *  data 都到位才动作一次——清除后 target 为 null，同一会话再次跳转时
 *  slice 存入新对象、effect 重新触发。 */
export function useSessionJumpConsumer(onOpen: (s: SessionRow) => void): void {
  const dispatch = useAppDispatch()
  const target = useAppSelector((s) => s.sessionJump.target)
  // target 字段名随会话复合主键（snake_case）；端点参数沿用 api 层的
  // camelCase 约定，在此映射。
  const { data } = useGetSessionQuery(
    target
      ? { id: target.id, deviceId: target.device_id }
      : { id: "", deviceId: "" },
    { skip: !target },
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — act once per (target, data); onOpen is an inline closure from useSessionsBrowser whose identity changes every render
  useEffect(() => {
    if (!target || data === undefined) return
    if (data) onOpen(data)
    dispatch(clearSessionJump())
  }, [target, data])
  // 落地视图卸载时仍未消费的 target 视为过期：跳转后用户立刻切走的话，
  // 不带这个清除，下次进会话视图会被陈旧 target 意外换开。跳转方（usage
  // 视图）与会话视图不同时挂载，此清除不会与新的跳转竞争。
  useEffect(
    () => () => {
      dispatch(clearSessionJump())
    },
    [dispatch],
  )
}
