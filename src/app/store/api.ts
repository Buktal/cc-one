import { createApi, fakeBaseQuery } from "@reduxjs/toolkit/query/react"
import type {
  AlignReport,
  App,
  AppError,
  AppInfo,
  CommonConfigSnippet,
  DeviceInfo,
  DeviceLibrarySummary,
  LibraryEntry,
  LibraryForgetAction,
  LocalGroup,
  LogsQuery,
  ModelStatsRow,
  PricingEntry,
  Provider,
  ProviderImportMode,
  ProviderImportReport,
  RunMode,
  SessionFilter,
  SessionGroup,
  SessionGroupCounts,
  SessionMessage_Serialize,
  SessionQuery,
  SessionRow,
  SyncedGroup,
  TrendBucket,
  TrendPoint,
  UploadItem,
  UsageFilter,
  UsageLogRow,
  UsageStats,
  VerifyReport,
} from "@/types/generated/bindings"
import {
  type CloseBehavior,
  commands,
  type Language,
  type LightweightExpand,
  type Preferences_Serialize,
  type Skin_Deserialize,
} from "@/types/generated/bindings"

/**
 * RTK Query data layer over the typed Tauri command contract.
 *
 * Every command returns a `{ status: "ok" | "error" }` envelope (tauri-specta).
 * `run` unwraps it into a discriminated `RunResult` — `{ data }` on ok or
 * `{ error: AppError }` on error — which queryFns return verbatim. RTK Query
 * stores a plain-object error returned from `queryFn` as-is (it only
 * serialises *thrown* Errors into `{ name, message, stack }`), so the typed
 * `AppError` reaches the UI intact and `describeError` can map `error.type`
 * to an i18n key. The UI never sees SQL or invoke() directly.
 *
 * `fakeBaseQuery<AppError>` pins the endpoint error type so `result.error` is
 * always `AppError` (not `unknown`) at call sites.
 */

type Envelope<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: AppError }

/** Outcome of `run`: the unwrapped payload, or the backend's typed error. */
export type RunResult<T> = { data: T } | { error: AppError }

async function run<T>(p: Promise<Envelope<T>>): Promise<RunResult<T>> {
  let r: Envelope<T>
  try {
    r = await p
  } catch (e) {
    // tauri-specta's `typedError` re-throws JS-level Errors (e.g. an invoke
    // failure or a Rust panic) instead of wrapping them in the envelope.
    // Normalise those into `Internal` so `run` never throws and the endpoint
    // error type stays honestly `AppError` — callers always get a typed result.
    return {
      error: {
        type: "Internal",
        data: e instanceof Error ? e.message : String(e),
      },
    }
  }
  if (r.status === "ok") return { data: r.data }
  return { error: r.error }
}

/** Stable cache id for a filter (so each filter scope caches independently). */
export function filterId(f: UsageFilter): string {
  return [f.from_ts, f.to_ts, f.model, f.source, f.device_scope].join("|")
}

/** Stable cache id for a SessionFilter (mirrors `filterId` for UsageFilter). */
export function sessionFilterId(f: SessionFilter): string {
  return [
    f.device_scope,
    f.source,
    f.favorited,
    f.local_group_id,
    f.synced_group_id,
    f.from_ts,
    f.to_ts,
    f.model,
    f.search,
  ].join("|")
}

/** Zero-value UsageStats — shared UI fallback for loading/empty. */
export const ZERO_STATS: UsageStats = {
  request_count: 0,
  total_tokens: 0,
  input_tokens: 0,
  output_tokens: 0,
  cache_creation_tokens: 0,
  cache_read_tokens: 0,
  cache_hit_rate: 0,
  total_cost_usd: 0,
  turn_count: 0,
  avg_turn_duration_ms: 0,
}

/** Empty (unconstrained) UsageFilter — for "any data at all" probes that must
 *  not narrow by the time / model / source / device window, e.g. deciding
 *  whether the source dimension should render at all. */
export const EMPTY_USAGE_FILTER: UsageFilter = {
  from_ts: null,
  to_ts: null,
  model: null,
  source: null,
  device_scope: null,
}

export const vaultApi = createApi({
  reducerPath: "vaultApi",
  baseQuery: fakeBaseQuery<AppError>(),
  tagTypes: [
    "Usage",
    "Logs",
    "Models",
    "Devices",
    "Pricing",
    "Library",
    "Sessions",
    "Providers",
    "App",
  ],
  endpoints: (b) => ({
    // ---- reads ----
    appInfo: b.query<AppInfo, void>({
      queryFn: async () => run(commands.getAppInfo()),
      providesTags: ["App"],
    }),
    stats: b.query<UsageStats, UsageFilter>({
      queryFn: async (filter) => run(commands.queryUsageStats(filter)),
      providesTags: (_r, _e, filter) => [
        { type: "Usage", id: filterId(filter) },
      ],
    }),
    trend: b.query<TrendPoint[], { filter: UsageFilter; bucket: TrendBucket }>({
      queryFn: async ({ filter, bucket }) =>
        run(commands.queryUsageTrend(filter, bucket)),
      providesTags: (_r, _e, { filter }) => [
        { type: "Usage", id: filterId(filter) },
      ],
    }),
    logs: b.query<UsageLogRow[], LogsQuery>({
      queryFn: async (q) => run(commands.queryUsageLogs(q)),
      providesTags: (_r, _e, q) => [{ type: "Logs", id: filterId(q.filter) }],
    }),
    count: b.query<number, UsageFilter>({
      queryFn: async (filter) => run(commands.countUsageLogs(filter)),
      providesTags: (_r, _e, filter) => [
        { type: "Logs", id: filterId(filter) },
      ],
    }),
    models: b.query<ModelStatsRow[], UsageFilter>({
      queryFn: async (filter) => run(commands.queryModels(filter)),
      providesTags: (_r, _e, filter) => [
        { type: "Models", id: filterId(filter) },
      ],
    }),
    distinctSources: b.query<string[], UsageFilter>({
      queryFn: async (filter) => run(commands.queryDistinctSources(filter)),
      providesTags: (_r, _e, filter) => [
        { type: "Usage", id: filterId(filter) },
      ],
    }),
    distinctModels: b.query<string[], UsageFilter>({
      queryFn: async (filter) => run(commands.queryDistinctModels(filter)),
      providesTags: (_r, _e, filter) => [
        { type: "Usage", id: filterId(filter) },
      ],
    }),
    devices: b.query<DeviceInfo[], void>({
      queryFn: async () => run(commands.listDevices()),
      providesTags: ["Devices"],
    }),
    pricing: b.query<PricingEntry[], void>({
      queryFn: async () => run(commands.listPricing()),
      providesTags: ["Pricing"],
    }),

    // ---- mutations ----
    collect: b.mutation<AlignReport, void>({
      queryFn: async () => run(commands.collectNow()),
      invalidatesTags: ["Usage", "Logs", "Models", "Devices"],
    }),
    sync: b.mutation<AlignReport, void>({
      queryFn: async () => run(commands.syncNow()),
      invalidatesTags: ["Usage", "Logs", "Models", "Devices"],
    }),
    rebill: b.mutation<number, void>({
      queryFn: async () => run(commands.rebillZeroCost()),
      invalidatesTags: ["Usage", "Logs", "Models"],
    }),

    // ---- pricing writes ----
    savePricing: b.mutation<
      null,
      { entry: PricingEntry; isBuiltin: boolean | null }
    >({
      queryFn: async ({ entry, isBuiltin }) =>
        run(commands.savePricingEntry(entry, isBuiltin)),
      invalidatesTags: ["Pricing"],
    }),
    deletePricing: b.mutation<null, string>({
      queryFn: async (modelKey) => run(commands.deletePricingEntry(modelKey)),
      invalidatesTags: ["Pricing"],
    }),
    reloadPricing: b.mutation<number, void>({
      queryFn: async () => run(commands.reloadPricingFromFile()),
      invalidatesTags: ["Pricing"],
    }),
    savePricingToFile: b.mutation<null, void>({
      queryFn: async () => run(commands.savePricingToFile()),
    }),
    fetchLitellm: b.mutation<number, void>({
      queryFn: async () => run(commands.fetchLitellmPricing()),
      invalidatesTags: ["Pricing"],
    }),

    // ---- library ----
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

    // ---- device / repo config ----
    setSyncRepo: b.mutation<RunMode, { repoUrl: string; githubToken: string }>({
      queryFn: async ({ repoUrl, githubToken }) =>
        run(commands.setSyncRepo(repoUrl, githubToken)),
      invalidatesTags: ["App"],
    }),
    verifySyncRepo: b.mutation<
      VerifyReport,
      { repoUrl: string | null; githubToken: string | null }
    >({
      queryFn: async ({ repoUrl, githubToken }) =>
        run(commands.verifySyncRepo(repoUrl, githubToken)),
      // Probe is read-only (ls-remote) — never invalidates any cache.
    }),
    clearSyncRepo: b.mutation<RunMode, void>({
      queryFn: async () => run(commands.clearSyncRepo()),
      invalidatesTags: ["App"],
    }),
    setDisplayName: b.mutation<null, string>({
      queryFn: async (displayName) => run(commands.setDisplayName(displayName)),
      invalidatesTags: ["App", "Devices"],
    }),
    setDeviceDisplayName: b.mutation<
      null,
      { deviceId: string; displayName: string }
    >({
      queryFn: async ({ deviceId, displayName }) =>
        run(commands.setDeviceDisplayName(deviceId, displayName)),
      invalidatesTags: ["Devices"],
    }),
    forgetDevice: b.mutation<
      null,
      { deviceId: string; libraryAction: LibraryForgetAction }
    >({
      queryFn: async ({ deviceId, libraryAction }) =>
        run(commands.forgetDevice(deviceId, libraryAction)),
      // "Library" too: migrate/delete rewrites the library listing.
      invalidatesTags: ["Devices", "Usage", "Logs", "Models", "Library"],
    }),

    // ---- sessions ----
    // Paged session list (mirrors `logs`: one query per filter+page, tagged by
    // filter only so a sessions_changed invalidate refetches every live page).
    // The filter carries the tab scope AND the sidebar selection (group id) —
    // group filtering moved backend-side so a group with many sessions pages
    // like the All view instead of loading all its rows.
    listSessions: b.query<SessionRow[], SessionQuery>({
      queryFn: async (query) => run(commands.querySessionsCmd(query)),
      providesTags: (_r, _e, query) => [
        {
          type: "Sessions",
          id: query.filter ? sessionFilterId(query.filter) : "all",
        },
      ],
    }),
    /** Sidebar + paginator counts for one grouping track: total (All row +
     *  page count) and per-bucket counts (group rows). Paging-independent —
     *  same filter as the page query, no limit/offset. */
    sessionCounts: b.query<
      SessionGroupCounts,
      { filter: SessionFilter | null; track: string }
    >({
      queryFn: async ({ filter, track }) =>
        run(commands.countSessionsCmd(filter, track)),
      providesTags: (_r, _e, { filter }) => [
        { type: "Sessions", id: filter ? sessionFilterId(filter) : "all" },
      ],
    }),
    /** One session's transcript (favorited-only — collect writes the JSONL only
     *  for favorited sessions). Cached per session. */
    sessionTranscript: b.query<
      SessionMessage_Serialize[],
      { id: string; deviceId: string }
    >({
      queryFn: async ({ id, deviceId }) =>
        run(commands.getSessionTranscriptCmd(id, deviceId)),
      providesTags: (_r, _e, { id, deviceId }) => [
        { type: "Sessions", id: `transcript:${deviceId}:${id}` },
      ],
    }),
    // Session user-data writes — every backend write emits `sessions_changed`,
    // which providers.tsx maps to a whole-`Sessions` tag invalidate (refetching
    // every active session query incl. the open transcript).
    setSessionFavorited: b.mutation<
      null,
      { id: string; deviceId: string; favorited: boolean }
    >({
      queryFn: async ({ id, deviceId, favorited }) =>
        run(commands.setSessionFavoritedCmd(id, deviceId, favorited)),
      invalidatesTags: ["Sessions"],
    }),
    setSessionCustomTitle: b.mutation<
      null,
      { id: string; deviceId: string; title: string | null }
    >({
      queryFn: async ({ id, deviceId, title }) =>
        run(commands.setSessionCustomTitleCmd(id, deviceId, title)),
      invalidatesTags: ["Sessions"],
    }),
    setSessionLocalGroup: b.mutation<
      null,
      { id: string; deviceId: string; groupId: string | null }
    >({
      queryFn: async ({ id, deviceId, groupId }) =>
        run(commands.setSessionLocalGroupCmd(id, deviceId, groupId)),
      invalidatesTags: ["Sessions"],
    }),
    setSessionSyncedGroup: b.mutation<
      null,
      { id: string; deviceId: string; groupId: string | null }
    >({
      queryFn: async ({ id, deviceId, groupId }) =>
        run(commands.setSessionSyncedGroupCmd(id, deviceId, groupId)),
      invalidatesTags: ["Sessions"],
    }),

    // Groups — unified list is the one the UI fetches; the per-track lists are
    // exposed for completeness. Both tracks cache under `Sessions` so any group
    // CRUD (which invalidates `Sessions`) refreshes the sidebar in place.
    listGroups: b.query<SessionGroup[], void>({
      queryFn: async () => run(commands.listGroupsCmd()),
      providesTags: ["Sessions"],
    }),
    listLocalGroups: b.query<LocalGroup[], void>({
      queryFn: async () => run(commands.listLocalGroupsCmd()),
      providesTags: ["Sessions"],
    }),
    listSyncedGroups: b.query<SyncedGroup[], void>({
      queryFn: async () => run(commands.listSyncedGroupsCmd()),
      providesTags: ["Sessions"],
    }),
    createLocalGroup: b.mutation<LocalGroup, string>({
      queryFn: async (name) => run(commands.createLocalGroupCmd(name)),
      invalidatesTags: ["Sessions"],
    }),
    renameLocalGroup: b.mutation<null, { id: string; name: string }>({
      queryFn: async ({ id, name }) =>
        run(commands.renameLocalGroupCmd(id, name)),
      invalidatesTags: ["Sessions"],
    }),
    deleteLocalGroup: b.mutation<null, string>({
      queryFn: async (id) => run(commands.deleteLocalGroupCmd(id)),
      invalidatesTags: ["Sessions"],
    }),
    createSyncedGroup: b.mutation<SyncedGroup, string>({
      queryFn: async (name) => run(commands.createSyncedGroupCmd(name)),
      invalidatesTags: ["Sessions"],
    }),
    renameSyncedGroup: b.mutation<null, { id: string; name: string }>({
      queryFn: async ({ id, name }) =>
        run(commands.renameSyncedGroupCmd(id, name)),
      invalidatesTags: ["Sessions"],
    }),
    deleteSyncedGroup: b.mutation<null, string>({
      queryFn: async (id) => run(commands.deleteSyncedGroupCmd(id)),
      invalidatesTags: ["Sessions"],
    }),
    // Drag-reorder: the full track order after a drop. Both tracks invalidate
    // `Sessions` like every other group write, so the sidebar refetches the
    // new order in place.
    reorderLocalGroups: b.mutation<null, string[]>({
      queryFn: async (orderedIds) =>
        run(commands.reorderLocalGroupsCmd(orderedIds)),
      invalidatesTags: ["Sessions"],
    }),
    reorderSyncedGroups: b.mutation<null, string[]>({
      queryFn: async (orderedIds) =>
        run(commands.reorderSyncedGroupsCmd(orderedIds)),
      invalidatesTags: ["Sessions"],
    }),

    // ---- providers ----
    // Provider CRUD (app-dimensioned: every query/mutation takes the app pool
    // it targets — the UI passes the current tab). Every write emits
    // `providers_changed`, which providers.tsx maps to a whole-`Providers` tag
    // invalidate; the endpoint-level invalidatesTags below cover writes that
    // go through the api anyway.
    listProviders: b.query<Provider[], App>({
      queryFn: async (app) => run(commands.listProvidersCmd(app)),
      providesTags: ["Providers"],
    }),
    /** 当前激活的完整 provider（「当前使用」光卡，按应用）；未激活/已删除 → null。 */
    getActiveProvider: b.query<Provider | null, App>({
      queryFn: async (app) => run(commands.getActiveProviderCmd(app)),
      providesTags: ["Providers"],
    }),
    saveProvider: b.mutation<Provider, Provider>({
      queryFn: async (provider) => run(commands.saveProviderCmd(provider)),
      invalidatesTags: ["Providers"],
    }),
    deleteProvider: b.mutation<null, { app: App; id: string }>({
      queryFn: async ({ app, id }) => run(commands.deleteProviderCmd(app, id)),
      invalidatesTags: ["Providers"],
    }),
    reorderProviders: b.mutation<null, { app: App; orderedIds: string[] }>({
      queryFn: async ({ app, orderedIds }) =>
        run(commands.reorderProvidersCmd(app, orderedIds)),
      invalidatesTags: ["Providers"],
    }),
    /** 切换供应商（按应用）：写盘 + 备份 + 记该应用的激活状态（只合并受控
     *  字段，非受控字段原地保留）。 */
    switchProvider: b.mutation<Provider, { app: App; id: string }>({
      queryFn: async ({ app, id }) => run(commands.switchProviderCmd(app, id)),
      invalidatesTags: ["Providers"],
    }),
    /** 附加模式（opencode）：把供应商写进 opencode.json 的 `provider.<key>`
     *  （与 switch 的 opencode 分支等价——多供应商共存、不取消其它、不记激活）。 */
    addProviderToLive: b.mutation<Provider, { app: App; id: string }>({
      queryFn: async ({ app, id }) =>
        run(commands.addProviderToLiveCmd(app, id)),
      invalidatesTags: ["Providers"],
    }),
    /** 附加模式（opencode）：从 opencode.json 删 `provider.<key>`（DB 记录保留，
     *  liveManaged=false，随时再加回来）。 */
    removeProviderFromLive: b.mutation<Provider, { app: App; id: string }>({
      queryFn: async ({ app, id }) =>
        run(commands.removeProviderFromLiveCmd(app, id)),
      invalidatesTags: ["Providers"],
    }),
    /** 附加模式（opencode）：把现有 opencode.json 的 `provider.*` 反向导入 DB。
     *  返回导入/更新条数。 */
    importProvidersFromLive: b.mutation<number, App>({
      queryFn: async (app) => run(commands.importProvidersFromLiveCmd(app)),
      invalidatesTags: ["Providers"],
    }),
    /** 某应用的通用配置片段（claude/codex/gemini 各一份）。 */
    getCommonConfigSnippet: b.query<CommonConfigSnippet, App>({
      queryFn: async (app) => run(commands.getCommonConfigSnippetCmd(app)),
      providesTags: ["Providers"],
    }),
    /** 保存某应用的通用配置片段（后端校验 JSON 合法性）。 */
    setCommonConfigSnippet: b.mutation<
      CommonConfigSnippet,
      { app: App; snippet: CommonConfigSnippet }
    >({
      queryFn: async ({ app, snippet }) =>
        run(
          commands.setCommonConfigSnippetCmd(
            app,
            snippet.content,
            snippet.enabled,
          ),
        ),
      invalidatesTags: ["Providers"],
    }),
    /** 导出全部供应商为 JSON 文档（`includeKeys=false` 剔除 API key）到用户
     *  选的路径。手动迁移 / 留档，不走 git 同步。返回文档内供应商数。 */
    exportProviders: b.mutation<
      number,
      { includeKeys: boolean; targetPath: string }
    >({
      queryFn: async ({ includeKeys, targetPath }) =>
        run(commands.exportProvidersCmd(includeKeys, targetPath)),
    }),
    /** 从用户选的 JSON 文档导入供应商（合并 / 覆盖模式）。只写本机 DB，
     *  不触发 providers.json 同步写。 */
    importProviders: b.mutation<
      ProviderImportReport,
      { sourcePath: string; mode: ProviderImportMode }
    >({
      queryFn: async ({ sourcePath, mode }) =>
        run(commands.importProvidersCmd(sourcePath, mode)),
      invalidatesTags: ["Providers"],
    }),
    /** 拉取供应商的模型列表（按应用分派端点格式；claude/codex 走 OpenAI
     *  兼容 GET /v1/models，后端发请求避免 WebView CORS）。失败时 error 是
     *  `FetchModels` 变体，data 为带分桶标签的错误串——表单按标签映射成
     *  对应 toast（model-fetch.ts 的 `bucketFetchModelsError`）。 */
    fetchModels: b.mutation<
      string[],
      { app: App; baseUrl: string; apiKey: string; modelsUrl: string | null }
    >({
      queryFn: async ({ app, baseUrl, apiKey, modelsUrl }) =>
        run(commands.fetchModelsCmd(app, baseUrl, apiKey, modelsUrl)),
    }),

    // ---- preferences ----
    // Go through the generated `commands.*` so tauri-specta's `typedError`
    // wrapping matches what `run` expects. Raw `invoke` skips that wrapping.
    preferences: b.query<Preferences_Serialize, void>({
      queryFn: async () => run(commands.getPreferences()),
      providesTags: ["App"],
    }),
    setCloseBehavior: b.mutation<Preferences_Serialize, CloseBehavior>({
      queryFn: async (closeBehavior) =>
        run(commands.setCloseBehavior(closeBehavior)),
      invalidatesTags: ["App"],
    }),
    setCollectInterval: b.mutation<Preferences_Serialize, number>({
      queryFn: async (seconds) => run(commands.setCollectInterval(seconds)),
      invalidatesTags: ["App"],
    }),
    setPushInterval: b.mutation<Preferences_Serialize, number>({
      queryFn: async (seconds) => run(commands.setPushInterval(seconds)),
      invalidatesTags: ["App"],
    }),
    setLanguage: b.mutation<Preferences_Serialize, Language>({
      queryFn: async (language) => run(commands.setLanguage(language)),
      invalidatesTags: ["App"],
    }),
    setLightweightExpand: b.mutation<Preferences_Serialize, LightweightExpand>({
      queryFn: async (mode) => run(commands.setLightweightExpand(mode)),
      invalidatesTags: ["App"],
    }),
    setSkin: b.mutation<Preferences_Serialize, Skin_Deserialize>({
      queryFn: async (skin) => run(commands.setSkin(skin)),
      invalidatesTags: ["App"],
    }),
  }),
})

export const {
  useAppInfoQuery,
  useStatsQuery,
  useTrendQuery,
  useLogsQuery,
  useCountQuery,
  useModelsQuery,
  useDistinctSourcesQuery,
  useDistinctModelsQuery,
  useDevicesQuery,
  usePricingQuery,
  useCollectMutation,
  useSyncMutation,
  useRebillMutation,
  useSavePricingMutation,
  useDeletePricingMutation,
  useReloadPricingMutation,
  useSavePricingToFileMutation,
  useFetchLitellmMutation,
  useScanLibraryQuery,
  useUploadToLibraryMutation,
  useExportFromLibraryMutation,
  useDeleteFromLibraryMutation,
  useRenameInLibraryMutation,
  useLibraryDeviceSummaryQuery,
  useLibraryTextQuery,
  useSetSyncRepoMutation,
  useVerifySyncRepoMutation,
  useClearSyncRepoMutation,
  useSetDisplayNameMutation,
  useSetDeviceDisplayNameMutation,
  useForgetDeviceMutation,
  usePreferencesQuery,
  useSetCloseBehaviorMutation,
  useSetCollectIntervalMutation,
  useSetPushIntervalMutation,
  useSetLanguageMutation,
  useSetLightweightExpandMutation,
  useSetSkinMutation,
  useListSessionsQuery,
  useSessionCountsQuery,
  useSessionTranscriptQuery,
  useSetSessionFavoritedMutation,
  useSetSessionCustomTitleMutation,
  useSetSessionLocalGroupMutation,
  useSetSessionSyncedGroupMutation,
  useListGroupsQuery,
  useListLocalGroupsQuery,
  useListSyncedGroupsQuery,
  useCreateLocalGroupMutation,
  useRenameLocalGroupMutation,
  useDeleteLocalGroupMutation,
  useCreateSyncedGroupMutation,
  useRenameSyncedGroupMutation,
  useDeleteSyncedGroupMutation,
  useReorderLocalGroupsMutation,
  useReorderSyncedGroupsMutation,
  useListProvidersQuery,
  useGetActiveProviderQuery,
  useSaveProviderMutation,
  useDeleteProviderMutation,
  useReorderProvidersMutation,
  useSwitchProviderMutation,
  useAddProviderToLiveMutation,
  useRemoveProviderFromLiveMutation,
  useImportProvidersFromLiveMutation,
  useGetCommonConfigSnippetQuery,
  useSetCommonConfigSnippetMutation,
  useExportProvidersMutation,
  useImportProvidersMutation,
  useFetchModelsMutation,
} = vaultApi

export type VaultApi = typeof vaultApi

/**
 * Resolve the one-time close dialog. Not an RTK Query endpoint —
 * it is a one-shot action (hide window / exit app). `remember` pins `choice`.
 * The sole caller fire-and-forgets this; on the rare error path the structured
 * `AppError` is thrown so a future caller could `describeError` it.
 */
export async function confirmClose(
  choice: CloseBehavior,
  remember: boolean,
): Promise<void> {
  const r = await run(commands.confirmClose(choice, remember))
  if ("error" in r) throw r.error
}
