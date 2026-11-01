import type { QueryClient } from "@tanstack/react-query";
import { linkOptions } from "@tanstack/react-router";

import { ActivityLog } from "@/features/audit/components/activity-log";
import { useAuditEntries } from "@/features/audit/hooks/audit-entries";
import { auditQueries, type AuditFilters } from "@/features/audit/lib/api";
import type { AuditEntityResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { useOptimisticSearch } from "@/hooks/optimistic-search";
import { useEntityPage } from "@/routes/_authenticated/activity/-entity-page";
import {
  ACTIVITY_SEARCH_DEFAULTS,
  type ActivitySearch,
} from "@/routes/_authenticated/activity/-search";

type ActivityEntity = Pick<AuditEntityResponse, "type" | "id">;

export async function loadEntityActivity(
  queryClient: QueryClient,
  entity: ActivityEntity,
  search: ActivitySearch,
) {
  await queryClient.infiniteQuery({
    ...auditQueries.list(activityQuery(entity, search)),
    staleTime: "static",
  });

  return { breadcrumb: m.audit_crumb() };
}

export function EntityActivityLog({
  entity,
  search,
  navigateToSearch,
}: {
  entity: ActivityEntity;
  search: ActivitySearch;
  navigateToSearch: (search: ActivitySearch) => Promise<void>;
}) {
  const { shownSearch, changeSearch, pending } = useOptimisticSearch(
    search,
    navigateToSearch,
  );
  const entityPage = useEntityPage();
  const auditEntries = useAuditEntries(activityQuery(entity, search));

  return (
    <ActivityLog
      filters={activityQuery(entity, shownSearch)}
      defaultFilters={activityQuery(entity, ACTIVITY_SEARCH_DEFAULTS)}
      onFiltersChange={(filters) =>
        changeSearch(activitySearchFromQuery(filters))
      }
      pending={pending}
      auditEntries={auditEntries}
      entityActions={(shownEntity) => ({
        filter:
          shownEntity.id === entity.id
            ? undefined
            : linkOptions({
                to: "/activity",
                search: {
                  ...ACTIVITY_SEARCH_DEFAULTS,
                  entity: {
                    type: shownEntity.type,
                    id: shownEntity.id,
                    name: shownEntity.name,
                  },
                },
              }),
        open: entityPage(shownEntity),
      })}
    />
  );
}

function activityQuery(
  { type, id }: ActivityEntity,
  { role, ...search }: ActivitySearch,
) {
  return { ...search, entity: { type, id, role } } satisfies AuditFilters;
}

function activitySearchFromQuery({ entity, from, to, order }: AuditFilters) {
  return {
    from,
    to,
    order,
    role: entity?.role ?? ACTIVITY_SEARCH_DEFAULTS.role,
  } satisfies ActivitySearch;
}
