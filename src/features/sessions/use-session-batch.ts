// Batch-operations domain of the sessions browser（架构审查Ⅴ拆分之二）：勾选
// 集状态（键 = favKey，值保留行定位——勾选可跨页留存）与批量收藏 / 归组 /
// 删除的编排。勾选迁移与结算口径住在 ./derive（withCheckedToggle /
// batchFailedCount——已测），本 hook 只把纯迁移接到 setState 上。
//
// 跨域依赖以显式参数进入：批量归组按轨道取对应 mutation（双轨分派表住在
// 分组域）——本域只依赖「一个可执行的归组动作」，组合根把
// groups.groupMutations(effectiveTrack).setGroup 包成本参数传入。

import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useDeleteSessionsMutation,
  useSetSessionFavoritedMutation,
} from "@/app/store/api"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import type { SessionRow } from "@/types/generated/bindings"
import {
  type BatchTarget,
  batchFailedCount,
  favKey,
  withCheckedToggle,
} from "./derive"

export interface SessionBatchDomainInput {
  /** 对一个勾选目标执行「归组」（track 分派已由注入方完成）。 */
  runSetGroup: (target: BatchTarget, groupId: string | null) => Promise<unknown>
}

export function useSessionBatch({ runSetGroup }: SessionBatchDomainInput) {
  const { t } = useTranslation()
  const [checked, setChecked] = useState<ReadonlyMap<string, BatchTarget>>(
    () => new Map(),
  )
  const [favoritedMut] = useSetSessionFavoritedMutation()
  const [deleteSessionsMut] = useDeleteSessionsMutation()
  const runWithToast = useMutateWithToast()

  function toggleCheck(s: SessionRow): void {
    setChecked((prev) => withCheckedToggle(prev, s))
  }
  function clearChecked(): void {
    setChecked(new Map())
  }
  function isChecked(s: SessionRow): boolean {
    return checked.has(favKey(s))
  }

  // ---- 批量操作（定稿 §6：勾选后批量收藏 / 归组 / 删除）----
  // 收藏/归组对全部勾选行并发执行，结束一条汇总 toast（逐行 toast 会在大
  // 勾选下刷屏）；删除走单条批量命令。
  async function runBatch(
    run: (target: BatchTarget) => Promise<unknown>,
    successKey: string,
  ): Promise<void> {
    const targets = [...checked.values()]
    const results = await Promise.allSettled(targets.map((t) => run(t)))
    const failed = batchFailedCount(results)
    if (failed === 0) {
      toast.success(t(successKey, { n: targets.length }))
    } else {
      toast.warning(t("sessions.toast.batchPartial", { n: failed }))
    }
    clearChecked()
  }

  async function batchFavorite(): Promise<void> {
    await runBatch(
      (t) => favoritedMut({ id: t.id, deviceId: t.device_id, favorited: true }),
      "sessions.toast.batchFavorited",
    )
  }

  async function batchSetGroup(groupId: string | null): Promise<void> {
    await runBatch(
      (t) => runSetGroup(t, groupId),
      "sessions.toast.batchGrouped",
    )
  }

  // 批量删除（#91）：一次命令带全部勾选键（后端批量软删除——排除标记随
  // 采集/拉取稳定，源文件不动）；确认对话在工具条（BatchBar）里，动作
  // 只在被确认后到达这里。返回值（实际命中行数）驱动成功 toast。
  async function batchDelete(): Promise<void> {
    const targets = [...checked.values()]
    if (targets.length === 0) return
    await runWithToast(deleteSessionsMut, targets, {
      success: {
        message: (n) => t("sessions.toast.batchDeleted", { n }),
      },
      failed: { key: "sessions.toast.failed" },
    })
    clearChecked()
  }

  return {
    checkedCount: checked.size,
    isChecked,
    toggleCheck,
    clearChecked,
    batchFavorite,
    batchSetGroup,
    batchDelete,
  }
}
