// Shared "collect now" action. Fires the align mutation (Standalone ⇒ local
// collect only; Synced ⇒ collect + pull + push), toasts the outcome, and stamps
// the data-freshness hint. Extracted here so the dashboard ControlCard, the
// request-log ControlBar, and the sessions view all trigger the exact same
// collect path — one concept, one implementation (architecture.md: 单一事实来源).
//
// The run mode decides what "collect" means; the button is always "refresh my
// data". `multiDevice` only tunes the success-toast wording (a sync that
// crossed devices vs a plain local collect); it does not change the mutation.

import { useTranslation } from "react-i18next"
import { useAppInfoQuery, useCollectMutation } from "@/app/store/api"
import { useFreshness } from "@/hooks/use-freshness"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"

/** 采集按钮的文案 key（单一归属——shell 侧栏 / 顶栏按钮与 request-log 空态
 *  CTA 共用）：collecting × multiDevice 两维——synced 模式的进行中文案是
 *  「同步中」而非「采集中」。空闲态文案由调用方注入（侧栏「采集 / 同步」，
 *  空态 CTA 用「采集本地日志」引导首次入库）。 */
export function collectLabelKey(
  collecting: boolean,
  multiDevice: boolean,
  idleKey: string,
): string {
  if (collecting) {
    return multiDevice ? "usage.collect.syncing" : "usage.collect.collecting"
  }
  return idleKey
}

export function useCollectAction(multiDevice: boolean) {
  const { t } = useTranslation()
  const { markCollected, markSynced } = useFreshness()
  // `collectNow` runs `align`: Standalone ⇒ local collect only; Synced ⇒
  // collect + pull + push. So a push actually happened iff the run mode is
  // Synced — gate the "synced" freshness stamp on that, not on the device
  // count (you can be Standalone with several discovered devices).
  const { data: info } = useAppInfoQuery(undefined, { pollingInterval: 0 })
  const synced = info?.mode === "synced"
  const [collect, { isLoading: collecting }] = useCollectMutation()
  const runWithToast = useMutateWithToast()
  async function onCollect() {
    const ok = await runWithToast(collect, undefined, {
      success: {
        message: (data) =>
          t(multiDevice ? "usage.collect.doneSync" : "usage.collect.done", {
            rows: data.collected.rows_inserted ?? 0,
            files: data.collected.files_scanned ?? 0,
          }),
      },
      failed: { key: "usage.collect.failed" },
    })
    if (!ok) return
    markCollected()
    if (synced) markSynced()
  }
  /** 采集按钮文案（侧栏版）：collecting × multiDevice 四态。 */
  const collectLabel = t(
    collectLabelKey(
      collecting,
      multiDevice,
      multiDevice ? "usage.collect.sync" : "usage.collect.collect",
    ),
  )
  return { onCollect, collecting, collectLabel }
}
