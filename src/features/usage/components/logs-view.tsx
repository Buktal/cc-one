// 请求日志视图. 顶部 ControlBar (时间/模型/刷新/采集)
// + 全宽日志表 (固定时间倒序)。查询条件与看板共享同一 filterSlice。

import { useAppSelector } from "@/app/store/hooks"

import { ControlBar } from "./control-card"
import { RequestLogTable } from "./request-log-table"

export function LogsView() {
  const filter = useAppSelector((s) => s.filter.filter)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <ControlBar />
      <RequestLogTable filter={filter} />
    </div>
  )
}
