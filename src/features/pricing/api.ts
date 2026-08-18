// Pricing-domain endpoints: the pricing table read + the pricing writes
// (entry CRUD, file reload/save, litellm fetch). Injected into vaultApi — the
// public hook seam is src/app/store/api.ts.

import { run, vaultApi } from "@/app/store/api-core"
import type { PricingEntry } from "@/types/generated/bindings"
import { commands } from "@/types/generated/bindings"

export const {
  usePricingQuery,
  useSavePricingMutation,
  useDeletePricingMutation,
  useReloadPricingMutation,
  useSavePricingToFileMutation,
  useFetchLitellmMutation,
} = vaultApi.injectEndpoints({
  endpoints: (b) => ({
    pricing: b.query<PricingEntry[], void>({
      queryFn: async () => run(commands.listPricing()),
      providesTags: ["Pricing"],
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
  }),
})
