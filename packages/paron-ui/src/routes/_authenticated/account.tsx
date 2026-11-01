import { useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";

import { CardHeader } from "@/components/card";
import { Page } from "@/components/route/page";
import { Card, CardContent, CardTitle } from "@/components/shadcn/card";
import { SignInMethods } from "@/features/account/components/sign-in-methods";
import { ProfileCard } from "@/features/users/components/profile-card";
import { RoleList } from "@/features/users/components/roles";
import { UserBadge } from "@/features/users/components/user-badge";
import { m } from "@/generated/paraglide/messages";
import { authQueries } from "@/lib/auth";

export const Route = createFileRoute("/_authenticated/account")({
  staticData: { width: "narrow" },
  loader: async ({ context }) => {
    await context.queryClient.query(authQueries.credentials());

    return { breadcrumb: m.account_crumb() };
  },
  component: RouteComponent,
});

function RouteComponent() {
  const { data: me } = useSuspenseQuery(authQueries.me());
  const { data: credentials } = useSuspenseQuery(authQueries.credentials());

  return (
    <Page title={<UserBadge user={me} size="lg" />}>
      <ProfileCard user={me} />

      <Card>
        <CardHeader>
          <CardTitle>{m.roles_title()}</CardTitle>
        </CardHeader>
        <CardContent>
          {me.roles.length === 0 ? (
            <p className="text-sm text-muted-foreground">{m.roles_none()}</p>
          ) : (
            <RoleList roles={me.roles} />
          )}
        </CardContent>
      </Card>

      <SignInMethods credentials={credentials} />
    </Page>
  );
}
