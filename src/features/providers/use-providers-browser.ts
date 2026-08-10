// Providers browser controller: the list query, drag-reorder wiring, and the
// export / import actions (manual migration — file dialogs + mutations + toasts
// live here, like the library browser's export; the dialogs are thin UI shells).
// `reorderIds` (lib/reorder.ts) owns the move math — this hook just feeds it
// the live order and ships the new one to `reorder_providers_cmd`. Save/delete
// stay in the view (they go through useMutateWithToast, like pricing).

import { open, save } from "@tauri-apps/plugin-dialog"
import { useTranslation } from "react-i18next"
import {
  useExportProvidersMutation,
  useImportProvidersMutation,
  useListProvidersQuery,
  useReorderProvidersMutation,
} from "@/app/store/api"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { reorderIds } from "@/lib/reorder"
import type { ProviderImportMode } from "@/types/generated/bindings"

export function useProvidersBrowser() {
  const { t } = useTranslation()
  const { data: providers = [], isLoading } = useListProvidersQuery()
  const [reorder] = useReorderProvidersMutation()
  const [exportMut, { isLoading: exporting }] = useExportProvidersMutation()
  const [importMut, { isLoading: importing }] = useImportProvidersMutation()
  const runWithToast = useMutateWithToast()

  /** Apply a drag move: recompute the order, skip the round trip when the
   *  move landed where it started. */
  function onReorder(activeId: string, overId: string): void {
    const next = reorderIds(
      providers.map((p) => p.id),
      activeId,
      overId,
    )
    if (next) void reorder(next)
  }

  /** Export all providers to a user-chosen path (save dialog), optionally
   *  including API keys. Canceled save dialog → no-op, returns false. */
  async function exportProviders(includeKeys: boolean): Promise<boolean> {
    const path = await save({
      defaultPath: "cc-one-providers.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    })
    if (!path) return false
    return runWithToast(
      exportMut,
      { includeKeys, targetPath: path },
      {
        success: {
          message: (count) => t("providers.toast.exported", { count }),
        },
        failed: { key: "providers.toast.exportFailed" },
      },
    )
  }

  /** Import providers from a user-chosen JSON file (open dialog) with the
   *  given conflict mode. Canceled open dialog → no-op, returns false. */
  async function importProviders(mode: ProviderImportMode): Promise<boolean> {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    })
    if (!path) return false
    return runWithToast(
      importMut,
      { sourcePath: String(path), mode },
      {
        success: {
          message: (r) =>
            t("providers.toast.imported", {
              imported: r.imported,
              skipped: r.skipped,
            }),
        },
        failed: { key: "providers.toast.importFailed" },
      },
    )
  }

  return {
    providers,
    isLoading,
    onReorder,
    exportProviders,
    importProviders,
    transferring: exporting || importing,
  }
}
