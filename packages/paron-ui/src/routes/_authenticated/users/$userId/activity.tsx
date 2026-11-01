import { useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute, stripSearchParams } from "@tanstack/react-router";

import { Page } from "@/components/route/page";
import { UserBadge } from "@/features/users/components/user-badge";
import { userQueries } from "@/features/users/lib/api";
import {
  EntityActivityLog,
  loadEntityActivity,
} from "@/routes/_authenticated/activity/-entity-activity";
import {
  ACTIVITY_SEARCH_DEFAULTS,
  activitySearch,
} from "@/routes/_authenticated/activity/-search";
import { UserTabs } from "@/routes/_authenticated/users/$userId/-tabs";

export const Route = createFileRoute("/_authenticated/users/$userId/activity")({
  staticData: { width: "narrow" },
  validateSearch: activitySearch,
  search: { middlewares: [stripSearchParams(ACTIVITY_SEARCH_DEFAULTS)] },
  loaderDeps: ({ search }) => search,
  loader: ({ context, params, deps }) =>
    loadEntityActivity(
      context.queryClient,
      { type: "user", id: params.userId },
      deps,
    ),
  component: RouteComponent,
});

function RouteComponent() {
  const { userId } = Route.useParams();
  const search = Route.useSearch();
  const navigate = Route.useNavigate();
  const { data: user } = useSuspenseQuery(userQueries.detail(userId));

  return (
    <Page title={<UserBadge user={user} showStatus size="lg" />}>
      <UserTabs />
      <EntityActivityLog
        entity={{ type: "user", id: userId }}
        search={search}
        navigateToSearch={(nextSearch) => navigate({ search: nextSearch })}
      />
    </Page>
  );
}
