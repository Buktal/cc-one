// Sessions-domain endpoints: the paged session list + counts + transcript,
// session writes, and the two-track group CRUD / reorder. Injected into
// vaultApi — the public hook seam is src/app/store/api.ts. The backend-filter
// assembly (buildSessionFilter) and the scope cache key (sessionSpecId) live
// in ./derive, so the filter knowledge stays with the feature.

import { run, vaultApi } from "@/app/store/api-core"
import { storeRead } from "@/app/store/tags"
import type {
  LocalGroup,
  SessionGroup,
  SessionGroupCounts,
  SessionMessage_Serialize,
  SessionRow,
  SyncedGroup,
} from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

import {
  buildSessionFilter,
  type SessionScopeSpec,
  sessionSpecId,
} from "./derive"

export const {
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
} = vaultApi.injectEndpoints({
  endpoints: (b) => ({
    // Paged session list (mirrors `logs`: one query per filter+page, tagged by
    // filter only so a sessions_changed invalidate refetches every live page).
    // The filter carries the tab scope AND the sidebar selection (group id) —
    // group filtering moved backend-side so a group with many sessions pages
    // like the All view instead of loading all its rows.
    listSessions: b.query<
      SessionRow[],
      SessionScopeSpec & { limit: number; offset: number }
    >({
      queryFn: async (q) => {
        const { limit, offset, ...scope } = q
        return run(
          commands.querySessionsCmd({
            filter: buildSessionFilter(scope),
            limit,
            offset,
          }),
        )
      },
      providesTags: (_r, _e, q) => {
        const { limit, offset, ...scope } = q
        return storeRead({ type: "Sessions", id: sessionSpecId(scope) })
      },
    }),
    /** Sidebar + paginator counts for one grouping track: total (All row +
     *  page count) and per-bucket counts (group rows). Paging-independent —
     *  same scope as the page query, no limit/offset. */
    sessionCounts: b.query<
      SessionGroupCounts,
      { spec: SessionScopeSpec; track: string }
    >({
      queryFn: async ({ spec, track }) =>
        run(commands.countSessionsCmd(buildSessionFilter(spec), track)),
      providesTags: (_r, _e, { spec }) =>
        storeRead({ type: "Sessions", id: sessionSpecId(spec) }),
    }),
    /** One session's transcript. Every session's messages land in the store
     *  (favorites additionally sync to git as snapshots); this reads the
     *  store, so it works for non-favorited sessions too. Cached per
     *  session. */
    sessionTranscript: b.query<
      SessionMessage_Serialize[],
      { id: string; deviceId: string }
    >({
      queryFn: async ({ id, deviceId }) =>
        run(commands.getSessionTranscriptCmd(id, deviceId)),
      providesTags: (_r, _e, { id, deviceId }) =>
        storeRead({ type: "Sessions", id: `transcript:${deviceId}:${id}` }),
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
      providesTags: storeRead("Sessions"),
    }),
    listLocalGroups: b.query<LocalGroup[], void>({
      queryFn: async () => run(commands.listLocalGroupsCmd()),
      providesTags: storeRead("Sessions"),
    }),
    listSyncedGroups: b.query<SyncedGroup[], void>({
      queryFn: async () => run(commands.listSyncedGroupsCmd()),
      providesTags: storeRead("Sessions"),
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
  }),
})
