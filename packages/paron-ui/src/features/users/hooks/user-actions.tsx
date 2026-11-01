import { useSuspenseQuery } from "@tanstack/react-query";
import { linkOptions } from "@tanstack/react-router";
import {
  CircleCheck,
  KeyRound,
  Pencil,
  Plus,
  Trash2,
  TriangleAlert,
} from "lucide-react";

import type { Action } from "@/components/actions";
import { useDialog } from "@/components/dialog";
import { errorToast } from "@/components/error-toast";
import { useToastManager } from "@/components/shadcn/toast";
import { AddUserDialog } from "@/features/users/components/add-user-dialog";
import { DeleteUserDialog } from "@/features/users/components/delete-user-dialog";
import { RecoveryLinkDialog } from "@/features/users/components/recovery-link-dialog";
import { StartRecoveryDialog } from "@/features/users/components/start-recovery-dialog";
import { SuspendUserDialog } from "@/features/users/components/suspend-user-dialog";
import { useStartRecovery, useUpdateUser } from "@/features/users/lib/api";
import type {
  RecoveryLinkResponse,
  UserResponse,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { usePermissions } from "@/hooks/permissions";
import { isApiError } from "@/lib/api-error";
import { authQueries } from "@/lib/auth";

export function useUserActions(
  user: UserResponse,
  { recoveryUrl }: { recoveryUrl: (issued: RecoveryLinkResponse) => string },
) {
  const dialog = useDialog();
  const permissions = usePermissions();
  const { data: me } = useSuspenseQuery(authQueries.me());
  const update = useUpdateUser();
  const toast = useToastManager();

  const canWrite = permissions.has("user:write");
  const manages = canWrite && me.id !== user.id;

  return {
    edit: canWrite
      ? {
          label: m.user_action_edit(),
          Icon: Pencil,
          kind: "link",
          link: linkOptions({
            to: "/users/$userId/edit",
            params: { userId: user.id },
          }),
        }
      : undefined,

    startRecovery:
      manages && user.status === "active"
        ? {
            label: m.user_action_start_recovery(),
            Icon: KeyRound,
            tone: "destructive",
            kind: "perform",
            perform: async () => {
              let issued: RecoveryLinkResponse | undefined;
              await dialog.open(
                <StartRecoveryDialog
                  user={user}
                  onStarted={(link) => (issued = link)}
                />,
              );
              if (!issued) return false;

              void dialog.open(
                <RecoveryLinkDialog
                  user={user}
                  url={recoveryUrl(issued)}
                  expiresAt={issued.expires_at}
                />,
              );
              return true;
            },
          }
        : undefined,

    suspend:
      manages && user.status === "active"
        ? {
            label: m.user_action_suspend(),
            Icon: TriangleAlert,
            tone: "warning",
            kind: "perform",
            perform: () => dialog.open(<SuspendUserDialog user={user} />),
          }
        : undefined,

    activate:
      manages && user.status !== "active"
        ? {
            label: m.user_action_activate(),
            Icon: CircleCheck,
            pending: update.isPending,
            kind: "perform",
            perform: async () => {
              try {
                await update.mutateAsync({
                  path: { id: user.id },
                  body: { status: "active" },
                });
              } catch (error) {
                if (!isApiError(error)) throw error;
                toast.add(errorToast(m.user_activate_error(), error));
                return false;
              }
              toast.add({
                type: "success",
                title: m.user_activate_success(),
                description: user.username,
              });
              return true;
            },
          }
        : undefined,

    delete: manages
      ? {
          label: m.user_action_delete(),
          Icon: Trash2,
          tone: "destructive",
          kind: "perform",
          perform: () => dialog.open(<DeleteUserDialog user={user} />),
        }
      : undefined,
  } satisfies Record<string, Action | undefined>;
}

export function useUsersActions({
  recoveryUrl,
}: {
  recoveryUrl: (issued: RecoveryLinkResponse) => string;
}) {
  const dialog = useDialog();
  const permissions = usePermissions();
  const startRecovery = useStartRecovery();
  const toast = useToastManager();

  return {
    add: permissions.has("user:write")
      ? {
          label: m.users_action_add(),
          Icon: Plus,
          pending: startRecovery.isPending,
          kind: "perform",
          perform: async () => {
            let created: UserResponse | undefined;
            await dialog.open(
              <AddUserDialog onCreated={(user) => (created = user)} />,
            );
            if (!created) return false;

            let issued: RecoveryLinkResponse;
            try {
              issued = await startRecovery.mutateAsync({
                path: { id: created.id },
              });
            } catch (error) {
              if (!isApiError(error)) throw error;
              toast.add(errorToast(m.user_start_recovery_error(), error));
              return true;
            }

            void dialog.open(
              <RecoveryLinkDialog
                user={created}
                url={recoveryUrl(issued)}
                expiresAt={issued.expires_at}
              />,
            );
            return true;
          },
        }
      : undefined,
  } satisfies Record<string, Action | undefined>;
}
