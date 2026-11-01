import {
  infiniteQueryOptions,
  type DefaultError,
  type InfiniteData,
} from "@tanstack/react-query";
import type { z } from "zod";

import { listAuditEntriesInfiniteQueryKey } from "@/generated/api/@tanstack/react-query.gen";
import { listAuditEntries } from "@/generated/api/sdk.gen";
import type { AuditPageResponse } from "@/generated/api/types.gen";
import type { zListAuditEntriesQuery } from "@/generated/api/zod.gen";

type AuditQuery = z.output<typeof zListAuditEntriesQuery>;

export type AuditFilters = Pick<AuditQuery, "entity" | "from" | "to" | "order">;
export type AuditRole = NonNullable<AuditFilters["entity"]>["role"];
export type AuditPages = InfiniteData<AuditPageResponse, AuditQuery["cursor"]>;

export const AUDIT_PAGE_SIZE = 50;

export const auditQueries = {
  // https://github.com/hey-api/hey-api/issues/2931
  list: (filters: AuditFilters) => {
    const queryWithLimit = { ...filters, limit: AUDIT_PAGE_SIZE };

    return infiniteQueryOptions<
      AuditPageResponse,
      DefaultError,
      AuditPages,
      ReturnType<typeof listAuditEntriesInfiniteQueryKey>,
      AuditQuery["cursor"]
    >({
      queryKey: listAuditEntriesInfiniteQueryKey({ query: queryWithLimit }),
      queryFn: async ({ pageParam, signal }) =>
        (
          await listAuditEntries({
            query: { ...queryWithLimit, cursor: pageParam },
            signal,
            throwOnError: true,
          })
        ).data,
      initialPageParam: undefined,
      getNextPageParam: ({ items }) =>
        items.length === AUDIT_PAGE_SIZE ? items.at(-1)?.id : undefined,
    });
  },
};
