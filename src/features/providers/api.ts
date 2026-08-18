// Providers-domain endpoints: provider CRUD (app-dimensioned), live-config
// attach/import, common-config snippets, TOML formatting, and provider import
// / export. Injected into vaultApi — the public hook seam is
// src/app/store/api.ts.

import { run, vaultApi } from "@/app/store/api-core"
import { storeRead } from "@/app/store/tags"
import type {
  App,
  CcSwitchImportReport,
  CommonConfigSnippet,
  LiveImportPreview,
  Provider,
  ProviderImportMode,
  ProviderImportReport,
} from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

export const {
  useListProvidersQuery,
  useGetActiveProviderQuery,
  useSaveProviderMutation,
  useDeleteProviderMutation,
  useReorderProvidersMutation,
  useSwitchProviderMutation,
  useAddProviderToLiveMutation,
  useRemoveProviderFromLiveMutation,
  useImportProvidersFromLiveMutation,
  usePreviewLiveImportMutation,
  useGetCommonConfigSnippetQuery,
  useSetCommonConfigSnippetMutation,
  useExtractSnippetFromLiveMutation,
  useFormatTomlMutation,
  useExportProvidersMutation,
  useImportProvidersMutation,
  useImportFromCcSwitchMutation,
  useFetchModelsMutation,
} = vaultApi.injectEndpoints({
  endpoints: (b) => ({
    // Provider CRUD (app-dimensioned: every query/mutation takes the app pool
    // it targets — the UI passes the current tab). Every write emits
    // `providers_changed`, which providers.tsx maps to a whole-`Providers` tag
    // invalidate; the endpoint-level invalidatesTags below cover writes that
    // go through the api anyway.
    listProviders: b.query<Provider[], App>({
      queryFn: async (app) => run(commands.listProvidersCmd(app)),
      providesTags: storeRead("Providers"),
    }),
    /** 当前激活的完整 provider（「当前使用」光卡，按应用）；未激活/已删除 → null。 */
    getActiveProvider: b.query<Provider | null, App>({
      queryFn: async (app) => run(commands.getActiveProviderCmd(app)),
      providesTags: storeRead("Providers"),
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
    /** 从 live 配置文件导入（ADR-0012 泛化，按 app 分派）：opencode 把现有
     *  opencode.json 的 `provider.*` 反向导入 DB；单激活应用（claude/codex/
     *  gemini/grok）读各自的 live 配置反向解析。返回导入/更新条数。
     *  `nameOverrides` = 预览列表里行内改过的名字（key → name，导入时优先于
     *  注册域推导 / entry.name）。 */
    importProvidersFromLive: b.mutation<
      number,
      { app: App; nameOverrides: Record<string, string> }
    >({
      queryFn: async ({ app, nameOverrides }) =>
        run(commands.importProvidersFromLiveCmd(app, nameOverrides)),
      invalidatesTags: ["Providers"],
    }),
    /** 附加模式（opencode）「从 opencode.json 导入」预览：只读，返回将导入的
     *  供应商（名称/端点/是否含密钥/新建或更新）；文件不存在 → Missing。 */
    previewLiveImport: b.mutation<LiveImportPreview, App>({
      queryFn: async (app) => run(commands.previewLiveImportCmd(app)),
    }),
    /** 某应用的通用配置片段（claude/codex/gemini 各一份）。 */
    getCommonConfigSnippet: b.query<CommonConfigSnippet, App>({
      queryFn: async (app) => run(commands.getCommonConfigSnippetCmd(app)),
      providesTags: storeRead("Providers"),
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
    /** 导入后「提取为通用片段」（T6）：读该应用 live 配置的可共享键，合并进
     *  现有片段（已有键不覆盖），启用片段。非静默——前端检测到候选先提示、
     *  用户确认才调。 */
    extractSnippetFromLive: b.mutation<CommonConfigSnippet, App>({
      queryFn: async (app) => run(commands.extractSnippetFromLiveCmd(app)),
      invalidatesTags: ["Providers"],
    }),
    /** TOML 片段「整理」（ADR-0011）：后端 taplo 格式化（保留注释与键序）。
     *  整理是容错的——失败保持原文，调用方不弹错误。 */
    formatToml: b.mutation<string, string>({
      queryFn: async (text) => run(commands.formatTomlCmd(text)),
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
    /** 从 CC-Switch 导入供应商：定位本机 CC-Switch 配置 → 翻译成 cc one
     *  Provider → 复用 apply_import 写库。代理 / OAuth / 不支持应用跳过并进报告。 */
    importFromCcSwitch: b.mutation<
      CcSwitchImportReport,
      { app: App; mode: ProviderImportMode; dbPath: string | null }
    >({
      queryFn: async ({ app, mode, dbPath }) =>
        run(commands.importFromCcswitchCmd(app, mode, dbPath)),
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
  }),
})
