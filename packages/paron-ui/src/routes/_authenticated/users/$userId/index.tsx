import { useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import {
  Fingerprint,
  KeyRound,
  ListChecks,
  ShieldOff,
  Smartphone,
} from "lucide-react";
import { Suspense } from "react";

import { ActionButton } from "@/components/actions";
import { CardAction, CardHeader } from "@/components/card";
import { DetailItem } from "@/components/detail-item";
import { Page } from "@/components/route/page";
import { Card, CardContent, CardTitle } from "@/components/shadcn/card";
import { Skeleton } from "@/components/shadcn/skeleton";
import { RoleList } from "@/features/users/components/roles";
import { UserBadge } from "@/features/users/components/user-badge";
import { useUserActions } from "@/features/users/hooks/user-actions";
import { userQueries } from "@/features/users/lib/api";
import { m } from "@/generated/paraglide/messages";
import { formatDateTime } from "@/lib/format";
import { useRecoveryUrl } from "@/routes/_auth/-recovery-link";
import { UserTabs } from "@/routes/_authenticated/users/$userId/-tabs";

export const Route = createFileRoute("/_authenticated/users/$userId/")({
  staticData: { width: "narrow" },
  component: RouteComponent,
});

function RouteComponent() {
  const { userId } = Route.useParams();
  const { data: user } = useSuspenseQuery(userQueries.detail(userId));
  const recoveryUrl = useRecoveryUrl();
  const actions = useUserActions(user, { recoveryUrl });
  const navigate = useNavigate();
  const accessActions = [
    actions.suspend,
    actions.activate,
    actions.startRecovery,
  ].filter((action) => action !== undefined);

  return (
    <Page
      title={<UserBadge user={user} showStatus size="lg" />}
      actions={actions.edit && <ActionButton action={actions.edit} />}
    >
      <UserTabs />
      <Card>
        <CardHeader>
          <CardTitle>{m.user_profile_title()}</CardTitle>
        </CardHeader>
        <CardContent>
          <dl className="grid grid-cols-1 gap-4 text-sm sm:grid-cols-3">
            <DetailItem term={m.user_username_label()}>
              {user.username}
            </DetailItem>
            <DetailItem term={m.user_display_name_label()}>
              {user.display_name}
            </DetailItem>
            <DetailItem term={m.user_created_label()}>
              {formatDateTime(user.created_at)}
            </DetailItem>
          </dl>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>{m.roles_title()}</CardTitle>
        </CardHeader>
        <CardContent>
          {user.roles.length === 0 ? (
            <p className="text-sm text-muted-foreground">{m.roles_none()}</p>
          ) : (
            <RoleList roles={user.roles} />
          )}
        </CardContent>
      </Card>

      {accessActions.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>{m.user_access_title()}</CardTitle>
            <CardAction>
              {accessActions.map((action) => (
                <ActionButton key={action.label} action={action} />
              ))}
            </CardAction>
          </CardHeader>
          <CardContent>
            <Suspense fallback={<Skeleton className="h-5 w-48" />}>
              <SignInMethodList userId={user.id} />
            </Suspense>
          </CardContent>
        </Card>
      )}

      {actions.delete && (
        <Card className="ring-destructive/40">
          <CardHeader>
            <CardTitle>{m.user_delete_card_title()}</CardTitle>
            <CardAction>
              <ActionButton
                action={actions.delete}
                onPerformed={() => void navigate({ to: "/users" })}
              />
            </CardAction>
          </CardHeader>
        </Card>
      )}
    </Page>
  );
}

function SignInMethodList({ userId }: { userId: string }) {
  const { data } = useSuspenseQuery(userQueries.credentials(userId));
  const remainingRecoveryCodes = data.recovery_codes?.remaining ?? 0;
  const methods = [
    data.password
      ? { Icon: KeyRound, label: m.user_sign_in_method_password() }
      : undefined,
    data.totp
      ? { Icon: Smartphone, label: m.user_sign_in_method_totp() }
      : undefined,
    data.passkeys.length > 0
      ? {
          Icon: Fingerprint,
          label: m.user_sign_in_method_passkey({ count: data.passkeys.length }),
        }
      : undefined,
    remainingRecoveryCodes > 0
      ? {
          Icon: ListChecks,
          label: m.user_sign_in_method_recovery_code({
            count: remainingRecoveryCodes,
          }),
        }
      : undefined,
  ].filter((method) => method !== undefined);

  if (methods.length === 0)
    return (
      <p className="flex items-center gap-2.5 text-sm text-muted-foreground">
        <ShieldOff className="size-4 shrink-0" />
        {m.user_sign_in_methods_none()}
      </p>
    );

  return (
    <ul className="grid grid-cols-1 gap-x-6 gap-y-2.5 text-sm sm:grid-cols-2">
      {methods.map(({ Icon, label }) => (
        <li key={label} className="flex items-center gap-2.5">
          <Icon className="size-4 shrink-0 text-muted-foreground" />
          {label}
        </li>
      ))}
    </ul>
  );
}
