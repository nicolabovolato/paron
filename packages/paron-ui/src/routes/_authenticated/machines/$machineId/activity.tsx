import { useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute, stripSearchParams } from "@tanstack/react-router";

import { Page } from "@/components/route/page";
import { machineQueries } from "@/features/machines/lib/api";
import {
  EntityActivityLog,
  loadEntityActivity,
} from "@/routes/_authenticated/activity/-entity-activity";
import {
  ACTIVITY_SEARCH_DEFAULTS,
  activitySearch,
} from "@/routes/_authenticated/activity/-search";
import { MachineTabs } from "@/routes/_authenticated/machines/$machineId/-tabs";

export const Route = createFileRoute(
  "/_authenticated/machines/$machineId/activity",
)({
  staticData: { width: "narrow" },
  validateSearch: activitySearch,
  search: { middlewares: [stripSearchParams(ACTIVITY_SEARCH_DEFAULTS)] },
  loaderDeps: ({ search }) => search,
  loader: ({ context, params, deps }) =>
    loadEntityActivity(
      context.queryClient,
      { type: "machine", id: params.machineId },
      deps,
    ),
  component: RouteComponent,
});

function RouteComponent() {
  const { machineId } = Route.useParams();
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  const { data: machine } = useSuspenseQuery(machineQueries.detail(machineId));

  return (
    <Page title={machine.name}>
      <MachineTabs />
      <EntityActivityLog
        entity={{ type: "machine", id: machineId }}
        search={search}
        navigateToSearch={(nextSearch) => navigate({ search: nextSearch })}
      />
    </Page>
  );
}
