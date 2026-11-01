import { useSuspenseQuery } from "@tanstack/react-query";
import {
  createFileRoute,
  linkOptions,
  useNavigate,
} from "@tanstack/react-router";

import { BackButton } from "@/components/route/back-button";
import { Page } from "@/components/route/page";
import { UserProfileForm } from "@/features/users/components/user-profile-form";
import { userQueries } from "@/features/users/lib/api";
import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/users/$userId/edit")({
  staticData: { permission: "user:write", width: "narrow" },
  loader: () => ({ breadcrumb: m.user_edit_crumb() }),
  component: RouteComponent,
});

function RouteComponent() {
  const { userId } = Route.useParams();
  const navigate = useNavigate();
  const { data: user } = useSuspenseQuery(userQueries.detail(userId));

  return (
    <Page title={m.user_edit_title({ name: user.display_name })}>
      <UserProfileForm
        user={user}
        cancel={
          <BackButton
            label={m.form_cancel()}
            variant="ghost"
            fallback={linkOptions({
              to: "/users/$userId",
              params: { userId },
            })}
          />
        }
        onSaved={() =>
          void navigate({ to: "/users/$userId", params: { userId } })
        }
      />
    </Page>
  );
}
