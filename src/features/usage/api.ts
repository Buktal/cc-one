// Usage-domain endpoints: dashboard reads (stats / trend / logs / models /
// distinct dimensions) and the collect / sync / rebill mutations. Injected
// into vaultApi — the public hook seam is src/app/store/api.ts. The usage
// cache key (filterId) lives in ./derive, built from the filterSlice dimension
// registry.

import { run, vaultApi } from "@/app/store/api-core"
import { type FilterState, toFilter } from "@/app/store/slices/filterSlice"
import { INVALIDATE_STORE, storeRead } from "@/app/store/tags"
import type {
  AlignReport,
  DeviceUsageRow,
  ModelStatsRow,
  ProjectCandidates,
  ProjectUsageRow,
  SessionUsageRow,
  TrendBucket,
  TrendPoint,
  UsageLogRow,
  UsageStats,
} from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

import { filterId } from "./derive"

/**
 * providesTags 样板收口（架构审查候选⑦顺带）：Store 域读端点的 tag 集合都是
 * 同一式——聚合 Store tag + 按 filterId 的细粒度 tag——仅 tag type 与
 * arg→filter 的取法不同。queryFn 的领域知识仍在各端点，这里只收机械三参。
 */
function readTags<Q>(
  type: "Usage" | "Logs" | "Models",
  filterOf: (arg: Q) => FilterState,
) {
  return (_r: unknown, _e: unknown, arg: Q) =>
    storeRead({ type, id: filterId(filterOf(arg)) })
}

export const {
  useStatsQuery,
  useTrendQuery,
  useLogsQuery,
  useCountQuery,
  useModelsQuery,
  useDistinctSourcesQuery,
  useDistinctModelsQuery,
  useDistinctProjectsQuery,
  useProjectUsageQuery,
  useSessionUsageQuery,
  useDeviceUsageQuery,
  useCollectMutation,
  useSyncMutation,
  useRebillMutation,
} = vaultApi.injectEndpoints({
  endpoints: (b) => ({
    // ---- reads ----
    stats: b.query<UsageStats, FilterState>({
      queryFn: async (filter) =>
        run(commands.queryUsageStats(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),
    trend: b.query<TrendPoint[], { filter: FilterState; bucket: TrendBucket }>({
      queryFn: async ({ filter, bucket }) =>
        run(commands.queryUsageTrend(toFilter(filter), bucket)),
      providesTags: readTags("Usage", ({ filter }) => filter),
    }),
    logs: b.query<
      UsageLogRow[],
      { filter: FilterState; limit: number; offset: number }
    >({
      queryFn: async (q) =>
        run(
          commands.queryUsageLogs({
            filter: toFilter(q.filter),
            limit: q.limit,
            offset: q.offset,
          }),
        ),
      providesTags: readTags("Logs", (q) => q.filter),
    }),
    count: b.query<number, FilterState>({
      queryFn: async (filter) => run(commands.countUsageLogs(toFilter(filter))),
      providesTags: readTags("Logs", (filter) => filter),
    }),
    models: b.query<ModelStatsRow[], FilterState>({
      queryFn: async (filter) => run(commands.queryModels(toFilter(filter))),
      providesTags: readTags("Models", (filter) => filter),
    }),
    distinctSources: b.query<string[], FilterState>({
      queryFn: async (filter) =>
        run(commands.queryDistinctSources(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),
    distinctModels: b.query<string[], FilterState>({
      queryFn: async (filter) =>
        run(commands.queryDistinctModels(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),
    // Project dropdown candidates (facet semantics). The unknown-project
    // sentinel rides as data (`unknown`), so the consumer labels the special
    // option without a second copy of the literal.
    distinctProjects: b.query<ProjectCandidates, FilterState>({
      queryFn: async (filter) =>
        run(commands.queryDistinctProjects(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),
    // Project buckets at usage grain (#106 dashboard project section) — sums
    // equal the stats totals under the same filter exactly.
    projectUsage: b.query<ProjectUsageRow[], FilterState>({
      queryFn: async (filter) =>
        run(commands.queryProjectUsage(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),
    // Session buckets at usage grain (#106 dashboard session section) — every
    // store-known session with its in-window usage + per-session turn counts.
    sessionUsage: b.query<SessionUsageRow[], FilterState>({
      queryFn: async (filter) =>
        run(commands.querySessionUsage(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),
    // Device buckets at usage grain (#107 dashboard device section) — pure
    // usage aggregates keyed by device id; naming / "this machine" identity
    // join frontend-side from listDevices.
    deviceUsage: b.query<DeviceUsageRow[], FilterState>({
      queryFn: async (filter) =>
        run(commands.queryDeviceUsage(toFilter(filter))),
      providesTags: readTags("Usage", (filter) => filter),
    }),

    // ---- mutations ----
    collect: b.mutation<AlignReport, void>({
      queryFn: async () => run(commands.collectNow()),
      invalidatesTags: INVALIDATE_STORE,
    }),
    sync: b.mutation<AlignReport, void>({
      queryFn: async () => run(commands.syncNow()),
      invalidatesTags: INVALIDATE_STORE,
    }),
    rebill: b.mutation<number, void>({
      queryFn: async () => run(commands.rebillZeroCost()),
      invalidatesTags: ["Usage", "Logs", "Models"],
    }),
  }),
})
