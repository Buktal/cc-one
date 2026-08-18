// SessionLink — 请求日志 DetailRow 的会话单元格：把 usage 行的 session_id
// 解析成会话标题，点击跳转会话视图并打开该会话（跨域通道见
// features/sessions/session-jump.ts）。解析未命中（null：无会话的历史用量、
// 已删除；undefined：加载中/出错）时退回裸 id 纯文本——与原展示一致，且
// 此时无会话可开，不可点。标题与 id 一起放 tooltip：id 是复制按钮拷走的
// 那个值，悬停可见才算「展示的与复制的是同一个东西」。

import { useGetSessionQuery } from "@/app/store/api"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useSessionJump } from "@/features/sessions/session-jump"

export function SessionLink({
  sessionId,
  deviceId,
}: {
  sessionId: string
  deviceId: string
}) {
  const jump = useSessionJump()
  const { data } = useGetSessionQuery({ id: sessionId, deviceId })

  // 未解析出会话行：保持裸 id 展示（原样、不可点）。
  if (!data) return <span className="truncate font-mono">{sessionId}</span>

  const title = data.title || sessionId
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            onClick={() => jump(sessionId, deviceId)}
            className="text-primary hover:text-primary/80 min-w-0 flex-1 truncate text-left"
          />
        }
      >
        {title}
      </TooltipTrigger>
      <TooltipContent>
        <div className="break-words">{title}</div>
        <div className="text-muted-foreground font-mono break-all">
          {sessionId}
        </div>
      </TooltipContent>
    </Tooltip>
  )
}
