// Public seam for the RTK Query data layer: every Tauri-command endpoint hook
// is exported from here, so callers import hooks from "@/app/store/api" and
// never from a feature api file directly.
//
// The base api instance + command plumbing live in ./api-core; the endpoints
// are injected per feature (src/features/*/api.ts). The re-export statements
// below evaluate those modules, so importing this file registers every
// endpoint on vaultApi before any hook is used.

// library
export {
  useDeleteFromLibraryMutation,
  useExportFromLibraryMutation,
  useLibraryDeviceSummaryQuery,
  useLibraryTextQuery,
  useRenameInLibraryMutation,
  useScanLibraryQuery,
  useUploadToLibraryMutation,
} from "@/features/library/api"
// pricing
export {
  useDeletePricingMutation,
  useFetchLitellmMutation,
  usePricingQuery,
  useReloadPricingMutation,
  useSavePricingMutation,
  useSavePricingToFileMutation,
} from "@/features/pricing/api"
// providers
export {
  useAddProviderToLiveMutation,
  useDeleteProviderMutation,
  useExportProvidersMutation,
  useExtractSnippetFromLiveMutation,
  useFetchModelsMutation,
  useFormatTomlMutation,
  useGetActiveProviderQuery,
  useGetCommonConfigSnippetQuery,
  useImportFromCcSwitchMutation,
  useImportProvidersFromLiveMutation,
  useImportProvidersMutation,
  useListProvidersQuery,
  usePreviewLiveImportMutation,
  useRemoveProviderFromLiveMutation,
  useReorderProvidersMutation,
  useSaveProviderMutation,
  useSetCommonConfigSnippetMutation,
  useSwitchProviderMutation,
} from "@/features/providers/api"
// sessions
export {
  useCreateLocalGroupMutation,
  useCreateSyncedGroupMutation,
  useDeleteLocalGroupMutation,
  useDeleteSyncedGroupMutation,
  useListGroupsQuery,
  useListLocalGroupsQuery,
  useListSessionsQuery,
  useListSyncedGroupsQuery,
  useRenameLocalGroupMutation,
  useRenameSyncedGroupMutation,
  useReorderLocalGroupsMutation,
  useReorderSyncedGroupsMutation,
  useSessionCountsQuery,
  useSessionTranscriptQuery,
  useSetSessionCustomTitleMutation,
  useSetSessionFavoritedMutation,
  useSetSessionLocalGroupMutation,
  useSetSessionSyncedGroupMutation,
} from "@/features/sessions/api"
// settings — app config: app info / device & repo / preferences
export {
  useAppInfoQuery,
  useClearSyncRepoMutation,
  useDevicesQuery,
  useForgetDeviceMutation,
  usePreferencesQuery,
  useSetCloseBehaviorMutation,
  useSetCollectIntervalMutation,
  useSetDeviceDisplayNameMutation,
  useSetDisplayNameMutation,
  useSetLanguageMutation,
  useSetLightweightAutoTuckMutation,
  useSetLightweightExpandMutation,
  useSetPushIntervalMutation,
  useSetSkinMutation,
  useSetSyncRepoMutation,
  useVerifySyncRepoMutation,
} from "@/features/settings/api"
// usage
export {
  useCollectMutation,
  useCountQuery,
  useDistinctModelsQuery,
  useDistinctSourcesQuery,
  useLogsQuery,
  useModelsQuery,
  useRebillMutation,
  useStatsQuery,
  useSyncMutation,
  useTrendQuery,
} from "@/features/usage/api"
export * from "./api-core"
