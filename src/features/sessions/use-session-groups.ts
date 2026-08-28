// Group domain of the sessions browser（架构审查Ⅴ拆分之三）：两个轨道
// （local = SQLite 直写，synced = git 往返）的分组 CRUD、轨道级归组动作、
// 双轨 mutation 分派表，以及乐观顺序覆盖（拖拽排序的写入在途时不回弹）。
// synced 组的创建需要绑定 Git 仓库（ADR 0002）——openCreateGroup 是 UX 守
// 卫（toast + 一跳到设置，不弹对话），createGroup 防御性复检兜底绕过入口的
// 调用方。
//
// 跨域依赖以显式参数进入：轨道选中状态归组合根（选中组被删时回「全部」的
// 复位动作以 onSelectedGroupDeleted 注入）；批量归组消费的分派表经
// groupMutations 暴露给组合根包装。

import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  useCreateLocalGroupMutation,
  useCreateSyncedGroupMutation,
  useDeleteLocalGroupMutation,
  useDeleteSyncedGroupMutation,
  useListGroupsQuery,
  useRenameLocalGroupMutation,
  useRenameSyncedGroupMutation,
  useReorderLocalGroupsMutation,
  useReorderSyncedGroupsMutation,
  useSetSessionLocalGroupMutation,
  useSetSessionSyncedGroupMutation,
} from "@/app/store/api"
import { useAppDispatch } from "@/app/store/hooks"
import { setView } from "@/app/store/slices/viewSlice"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import type {
  GroupTrack,
  SessionGroup,
  SessionRow,
} from "@/types/generated/bindings"
import { applyGroupOrder, canCreateSyncedGroup } from "./derive"

export interface SessionGroupsDomainInput {
  /** 当前轨道（local / favorites 宇宙的双轨语言）。 */
  effectiveTrack: GroupTrack
  /** git 同步开关（appInfo）——synced 组创建的守卫条件。 */
  synced: boolean
  /** 当前选中组：删除的正是它时复位（见 onSelectedGroupDeleted）。 */
  selectedGroupId: string
  /** 选中组被删除后的复位动作（组合根的 setSelectedGroupId(ALL_GROUPS)）。 */
  onSelectedGroupDeleted: () => void
}

export function useSessionGroups({
  effectiveTrack,
  synced,
  selectedGroupId,
  onSelectedGroupDeleted,
}: SessionGroupsDomainInput) {
  const { t } = useTranslation()
  const dispatch = useAppDispatch()
  const [createGroupOpen, setCreateGroupOpen] = useState(false)
  const [pendingGroup, setPendingGroup] = useState<string | null>(null)
  const [busyGroupId, setBusyGroupId] = useState<string | null>(null)
  // Same pattern as the favorite override: the group-drag override is cleared
  // when the reorder's invalidation delivers the real order.
  const [groupOrderOverride, setGroupOrderOverride] = useState<string[] | null>(
    null,
  )

  const groupsQuery = useListGroupsQuery()
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — clear overrides when fresh query data arrives; the body needs no groupsData value
  useEffect(() => {
    setGroupOrderOverride(null)
  }, [groupsQuery.data])

  // Natural order comes sorted from the backend; the override re-sorts it
  // optimistically while a drag's write is in flight.
  const trackGroups = useMemo(
    () =>
      applyGroupOrder(
        (groupsQuery.data ?? []).filter((g) => g.kind === effectiveTrack),
        groupOrderOverride,
      ),
    [groupsQuery.data, effectiveTrack, groupOrderOverride],
  )

  const [setLocalGroupMut] = useSetSessionLocalGroupMutation()
  const [setSyncedGroupMut] = useSetSessionSyncedGroupMutation()
  const [createLocalMut] = useCreateLocalGroupMutation()
  const [renameLocalMut] = useRenameLocalGroupMutation()
  const [deleteLocalMut] = useDeleteLocalGroupMutation()
  const [createSyncedMut] = useCreateSyncedGroupMutation()
  const [renameSyncedMut] = useRenameSyncedGroupMutation()
  const [deleteSyncedMut] = useDeleteSyncedGroupMutation()
  const [reorderLocalMut] = useReorderLocalGroupsMutation()
  const [reorderSyncedMut] = useReorderSyncedGroupsMutation()
  const runWithToast = useMutateWithToast()

  /** group 双轨 mutation 分派表（架构扫描候选⑨b 收编为表驱动）：local
   *  （SQLite 直写）vs synced（git 往返）——四个 group mutation 的成对关系
   *  一张表穷尽（Record<GroupTrack, …> 编译期锁），调用方只选轨道：操作当
   *  前标签的组（setSessionGroup / reorderGroups / 批量归组）传
   *  effectiveTrack；操作指定组（renameGroup / deleteGroup）传该组的 kind。
   *  create 不在表里：其返回类型两轨不同（LocalGroup vs SyncedGroup），
   *  union trigger 无法喂给 runWithToast 的泛型推断，createGroup 保留手动
   *  分支。新增 group mutation 时成对关系必须进这张表。 */
  function groupMutations(track: GroupTrack) {
    const table = {
      local: {
        setGroup: setLocalGroupMut,
        rename: renameLocalMut,
        delete: deleteLocalMut,
        reorder: reorderLocalMut,
      },
      synced: {
        setGroup: setSyncedGroupMut,
        rename: renameSyncedMut,
        delete: deleteSyncedMut,
        reorder: reorderSyncedMut,
      },
    } as const
    return table[track]
  }

  // ---- session row action: assign to a group on the current track ----
  async function setSessionGroup(
    s: SessionRow,
    groupId: string | null,
  ): Promise<void> {
    const mut = groupMutations(effectiveTrack).setGroup
    await runWithToast(
      mut,
      { id: s.id, deviceId: s.device_id, groupId },
      {
        success: { key: "sessions.toast.groupAssigned" },
        failed: { key: "sessions.toast.failed" },
      },
    )
  }

  function notifyGitRequired(): void {
    toast.warning(t("sessions.group.gitRequiredTitle"), {
      description: t("sessions.group.gitRequiredDesc"),
      action: {
        label: t("sessions.group.gitRequiredAction"),
        onClick: () => dispatch(setView("settings")),
      },
    })
  }

  function openCreateGroup(): void {
    if (!canCreateSyncedGroup(effectiveTrack, synced)) {
      notifyGitRequired()
      return
    }
    setCreateGroupOpen(true)
  }

  async function createGroup(name: string): Promise<boolean> {
    const trimmed = name.trim()
    if (!trimmed) return false
    if (!canCreateSyncedGroup(effectiveTrack, synced)) {
      notifyGitRequired()
      return false
    }
    // create 的返回类型两轨不同（LocalGroup vs SyncedGroup），union trigger
    // 无法喂给 runWithToast 的泛型推断（TS 逆变的 trigger 参数不接受联合），
    // 分支保留在此（groupMutations 表里无 create 条目，见其注释）。
    setPendingGroup(trimmed)
    const ok =
      effectiveTrack === "local"
        ? await runWithToast(createLocalMut, trimmed, {
            success: { key: "sessions.toast.groupCreated" },
            failed: { key: "sessions.toast.failed" },
          })
        : await runWithToast(createSyncedMut, trimmed, {
            success: { key: "sessions.toast.groupCreated" },
            failed: { key: "sessions.toast.failed" },
          })
    setPendingGroup(null)
    if (ok) setCreateGroupOpen(false)
    return ok
  }

  async function renameGroup(g: SessionGroup, name: string): Promise<void> {
    const trimmed = name.trim()
    if (!trimmed || trimmed === g.name) return
    setBusyGroupId(g.id)
    try {
      // 组自身的轨道决定走哪套 mutation（groups.json 里来的 synced 组与本地
      // 组共存于同一侧栏）。
      const mut = groupMutations(g.kind === "local" ? "local" : "synced").rename
      await runWithToast(
        mut,
        { id: g.id, name: trimmed },
        {
          success: { key: "sessions.toast.groupRenamed" },
          failed: { key: "sessions.toast.failed" },
        },
      )
    } finally {
      setBusyGroupId(null)
    }
  }

  async function deleteGroup(g: SessionGroup): Promise<void> {
    setBusyGroupId(g.id)
    try {
      const mut = groupMutations(g.kind === "local" ? "local" : "synced").delete
      const ok = await runWithToast(mut, g.id, {
        success: { key: "sessions.toast.groupDeleted" },
        failed: { key: "sessions.toast.failed" },
      })
      if (ok && selectedGroupId === g.id) onSelectedGroupDeleted()
    } finally {
      setBusyGroupId(null)
    }
  }

  // Group drag-reorder: optimistic stamp → mutate → snap back on failure. A
  // drag must not visibly snap while the write is in flight (synced reorders
  // round-trip through git), and the outcome is already visible to the user —
  // no success toast.
  async function reorderGroups(orderedIds: string[]): Promise<void> {
    setGroupOrderOverride(orderedIds)
    const mut = groupMutations(effectiveTrack).reorder
    const ok = await runWithToast(mut, orderedIds, {
      failed: { key: "sessions.toast.failed" },
    })
    if (!ok) setGroupOrderOverride(null)
  }

  return {
    trackGroups,
    createGroupOpen,
    setCreateGroupOpen,
    openCreateGroup,
    createGroup,
    renameGroup,
    deleteGroup,
    reorderGroups,
    pendingGroup,
    busyGroupId,
    setSessionGroup,
    groupMutations,
  }
}

export type SessionGroupsDomain = ReturnType<typeof useSessionGroups>
