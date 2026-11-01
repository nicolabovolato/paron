import { useSuspenseInfiniteQuery } from "@tanstack/react-query";

import {
  auditQueries,
  type AuditFilters,
  type AuditPages,
} from "@/features/audit/lib/api";

export function useAuditEntries(filters: AuditFilters) {
  const result = useSuspenseInfiniteQuery({
    ...auditQueries.list(filters),
    select: entriesOfAllPages,
  });

  return {
    entries: result.data,
    moreEntries: result.hasNextPage,
    loadingMore: result.isFetchingNextPage,
    loadMoreFailed: result.isFetchNextPageError,
    loadMore: () => void result.fetchNextPage(),
  };
}

function entriesOfAllPages({ pages }: AuditPages) {
  return pages.flatMap((page) => page.items);
}

export type AuditEntries = ReturnType<typeof useAuditEntries>;
