// Settings-domain endpoints — the app's own config surface: app info, device /
// repo config (incl. the forget-device write), and the preferences reads /
// writes. Injected into vaultApi — the public hook seam is src/app/store/api.ts.

import { run, vaultApi } from "@/app/store/api-core"
import { INVALIDATE_STORE, storeRead } from "@/app/store/tags"
import type {
  AppInfo,
  CloseBehavior,
  DeviceInfo,
  Language,
  LibraryForgetAction,
  LightweightExpand,
  Preferences_Serialize,
  RunMode,
  Skin_Deserialize,
  VerifyReport,
} from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

export const {
  useAppInfoQuery,
  useDevicesQuery,
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
  useSetLightweightAutoTuckMutation,
  useSetSkinMutation,
} = vaultApi.injectEndpoints({
  endpoints: (b) => ({
    // ---- reads ----
    appInfo: b.query<AppInfo, void>({
      queryFn: async () => run(commands.getAppInfo()),
      providesTags: ["App"],
    }),
    devices: b.query<DeviceInfo[], void>({
      queryFn: async () => run(commands.listDevices()),
      providesTags: storeRead("Devices"),
    }),

    // ---- device / repo config ----
    setSyncRepo: b.mutation<RunMode, { repoUrl: string; githubToken: string }>({
      queryFn: async ({ repoUrl, githubToken }) =>
        run(commands.setSyncRepo(repoUrl, githubToken)),
      invalidatesTags: ["App", ...INVALIDATE_STORE],
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
      invalidatesTags: [...INVALIDATE_STORE, "Library"],
    }),

    // ---- preferences ----
    // Go through the generated `commands.*` so tauri-specta's `typedError`
    // wrapping matches what `run` expects. Raw `invoke` skips that wrapping.
    //
    // 失效约定（config 轨）：preferences 写命令只落本机 config.json，不走
    // Tauri 事件总线（没有 preferences_changed）——写后失效靠每个 mutation
    // 手写 `invalidatesTags: ["App"]` 补偿（appInfo / preferences 两个读都
    // provide "App"）。事件方案（Rust 统一 emit + 全局监听）评估过：为纯
    // 本机写引入一条新事件比这八个字面量更重，未采用。新增 preferences 写
    // 命令必须带同样的 invalidatesTags: ["App"]，漏写即 UI 显示旧值。
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
    setLightweightAutoTuck: b.mutation<Preferences_Serialize, number>({
      queryFn: async (secs) => run(commands.setLightweightAutoTuck(secs)),
      invalidatesTags: ["App"],
    }),
    setSkin: b.mutation<Preferences_Serialize, Skin_Deserialize>({
      queryFn: async (skin) => run(commands.setSkin(skin)),
      invalidatesTags: ["App"],
    }),
  }),
})
