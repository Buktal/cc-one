// Library-domain endpoints: scan / upload / export / delete / rename over the
// device library, plus the forget-device probe and the text preview. Injected
// into vaultApi — the public hook seam is src/app/store/api.ts.

import { run, vaultApi } from "@/app/store/api-core"
import type {
  DeviceLibrarySummary,
  LibraryEntry,
  UploadItem,
} from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

export const {
  useScanLibraryQuery,
  useUploadToLibraryMutation,
  useExportFromLibraryMutation,
  useDeleteFromLibraryMutation,
  useRenameInLibraryMutation,
  useLibraryDeviceSummaryQuery,
  useLibraryTextQuery,
} = vaultApi.injectEndpoints({
  endpoints: (b) => ({
    scanLibrary: b.query<
      LibraryEntry[],
      { deviceScope: string; subpath: string }
    >({
      queryFn: async ({ deviceScope, subpath }) =>
        run(commands.scanLibrary(deviceScope, subpath)),
      providesTags: ["Library"],
    }),
    uploadToLibrary: b.mutation<null, { items: UploadItem[]; subpath: string }>(
      {
        queryFn: async ({ items, subpath }) =>
          run(commands.uploadToLibrary(items, subpath)),
        invalidatesTags: ["Library"],
      },
    ),
    exportFromLibrary: b.mutation<null, { relPath: string; targetDir: string }>(
      {
        queryFn: async ({ relPath, targetDir }) =>
          run(commands.exportFromLibrary(relPath, targetDir)),
      },
    ),
    deleteFromLibrary: b.mutation<null, string>({
      queryFn: async (relPath) => run(commands.deleteFromLibrary(relPath)),
      invalidatesTags: ["Library"],
    }),
    renameInLibrary: b.mutation<null, { relPath: string; newName: string }>({
      queryFn: async ({ relPath, newName }) =>
        run(commands.renameInLibrary(relPath, newName)),
      invalidatesTags: ["Library"],
    }),
    /** Pre-flight file/folder counts for one device's library subtree — drives
     *  the forget-device dialog's migrate-vs-delete choice. Read-only probe. */
    libraryDeviceSummary: b.query<DeviceLibrarySummary, string>({
      queryFn: async (deviceId) => run(commands.libraryDeviceSummary(deviceId)),
    }),
    /** Themed text preview: `null` = not text (binary / over the size cap). */
    libraryText: b.query<string | null, string>({
      queryFn: async (relPath) => run(commands.readLibraryText(relPath)),
    }),
  }),
})
