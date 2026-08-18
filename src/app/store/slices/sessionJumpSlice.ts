// 会话跳转通道：其他域（当前唯一跳转方 = usage 请求日志的 session 单元格）
// 请求「切到会话视图并打开这个会话」的最小全局状态。现有的打开会话状态
// （previewKey）是 SessionsView 内 useSessionsBrowser 的局部 state，跨视图
// 不可达，所以这里只挂一个一次性 target：跳转方写入（同时 setView），挂在
// SessionsView 里的 consumer（features/sessions/session-jump.ts）取回会话行
// 后经 setPreview 打开并清除 target。slice 本身不含数据查询——会话行的取回
// 走 sessions api 的 getSession 端点（RTK 缓存，与跳转方的标题显示共用同一
// 行）。不持久化：跳转是即时动作，重启后不应残留。

import { createSlice } from "@reduxjs/toolkit"

/** 跳转目标 — 会话的复合主键，与 usage 记录的 (session_id, device_id) 同形。 */
export interface SessionJumpTarget {
  id: string
  device_id: string
}

interface SessionJumpState {
  target: SessionJumpTarget | null
}

const initialState: SessionJumpState = {
  target: null,
}

const sessionJumpSlice = createSlice({
  name: "sessionJump",
  initialState,
  reducers: {
    setSessionJumpTarget(state, action: { payload: SessionJumpTarget }) {
      state.target = action.payload
    },
    clearSessionJump(state) {
      state.target = null
    },
  },
})

export const { setSessionJumpTarget, clearSessionJump } =
  sessionJumpSlice.actions
export default sessionJumpSlice.reducer
