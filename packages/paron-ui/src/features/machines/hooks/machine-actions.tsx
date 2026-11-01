import { linkOptions } from "@tanstack/react-router";
import {
  Pencil,
  PlugZap,
  Plus,
  SquareTerminal,
  Ticket,
  Trash2,
  Unlink,
} from "lucide-react";

import type { Action } from "@/components/actions";
import { useDialog } from "@/components/dialog";
import { errorToast } from "@/components/error-toast";
import { useToastManager } from "@/components/shadcn/toast";
import { DeleteMachineDialog } from "@/features/machines/components/delete-machine-dialog";
import { CancelTunnelSetupDialog } from "@/features/machines/components/tunnel/cancel-tunnel-setup-dialog";
import { RevokeTunnelAccessDialog } from "@/features/machines/components/tunnel/revoke-tunnel-access-dialog";
import { TunnelSetupDialog } from "@/features/machines/components/tunnel/tunnel-setup-dialog";
import { useConnectionTest } from "@/features/machines/hooks/connection-test";
import { useIssueMachineEnrollmentToken } from "@/features/machines/lib/api";
import {
  isExpired,
  testConnectionDisabled,
} from "@/features/machines/lib/utils";
import type { MachineResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { useNow } from "@/hooks/now";
import { usePermissions } from "@/hooks/permissions";
import { isApiError } from "@/lib/api-error";

export function useMachineActions(machine: MachineResponse) {
  const dialog = useDialog();
  const permissions = usePermissions();
  const enrollment =
    machine.connectivity.kind === "tunneled"
      ? machine.connectivity.enrollment
      : undefined;
  const expired = useNow(
    (now) => enrollment !== undefined && isExpired(enrollment, now),
  );
  const enrollmentState = expired ? "expired" : enrollment?.state;
  const issueToken = useIssueMachineEnrollmentToken();
  const toast = useToastManager();
  const connectionTest = useConnectionTest(machine.id);
  const [connectionTestProvider] = machine.capabilities.connection_test;
  const [terminalProvider] = machine.capabilities.terminal;
  const connectionTestProtocol = connectionTestProvider?.protocol;
  const terminalProtocol = terminalProvider?.protocol;

  const enrolls = permissions.has("machine:enroll");

  return {
    testConnection: !permissions.has("machine:operate")
      ? undefined
      : connectionTestProtocol
        ? {
            label: m.connection_test_action(),
            Icon: PlugZap,
            disabled: testConnectionDisabled({
              tunneled: enrollment !== undefined,
              enrolled: enrollmentState === "enrolled",
            }),
            pending: connectionTest.pending,
            kind: "perform",
            perform: async () =>
              (
                await connectionTest.saved(machine.name, {
                  path: { id: machine.id },
                  query: { protocol: connectionTestProtocol },
                })
              ).ok,
          }
        : {
            label: m.connection_test_action(),
            Icon: PlugZap,
            disabled: { reason: m.connection_test_disabled_no_ssh() },
            kind: "perform",
            perform: () => Promise.resolve(false),
          },

    openTerminal: !permissions.has("machine:operate")
      ? undefined
      : terminalProtocol
        ? {
            label: m.machine_action_open_terminal(),
            Icon: SquareTerminal,
            disabled: enrollment !== undefined &&
              enrollmentState !== "enrolled" && {
                reason: m.terminal_disabled_not_enrolled(),
              },
            kind: "link",
            link: linkOptions({
              to: "/machines/$machineId/terminal",
              params: { machineId: machine.id },
              search: { protocol: terminalProtocol },
            }),
          }
        : {
            label: m.machine_action_open_terminal(),
            Icon: SquareTerminal,
            disabled: { reason: m.terminal_disabled_no_ssh() },
            kind: "perform",
            perform: () => Promise.resolve(false),
          },

    edit: permissions.has("machine:write")
      ? {
          label: m.machine_action_edit(),
          Icon: Pencil,
          kind: "link",
          link: linkOptions({
            to: "/machines/$machineId/edit",
            params: { machineId: machine.id },
          }),
        }
      : undefined,

    setUpTunnel:
      enrolls &&
      (enrollmentState === "not_enrolled" || enrollmentState === "expired")
        ? {
            label: m.tunnel_action_set_up(),
            Icon: Ticket,
            pending: issueToken.isPending,
            kind: "perform",
            perform: async () => {
              try {
                const token = await issueToken.mutateAsync({
                  path: { id: machine.id },
                });
                void dialog.open(
                  <TunnelSetupDialog machine={machine} token={token} />,
                );
                return true;
              } catch (error) {
                if (!isApiError(error)) throw error;
                toast.add(errorToast(m.tunnel_set_up_error(), error));
                return false;
              }
            },
          }
        : undefined,

    cancelTunnelSetup:
      enrolls && enrollmentState === "invited"
        ? {
            label: m.tunnel_action_cancel(),
            Icon: Unlink,
            kind: "perform",
            perform: () =>
              dialog.open(<CancelTunnelSetupDialog machine={machine} />),
          }
        : undefined,

    revokeTunnelAccess:
      enrolls && enrollmentState === "enrolled"
        ? {
            label: m.tunnel_action_revoke(),
            Icon: Unlink,
            kind: "perform",
            perform: () =>
              dialog.open(<RevokeTunnelAccessDialog machine={machine} />),
          }
        : undefined,

    delete: permissions.has("machine:delete")
      ? {
          label: m.machine_action_delete(),
          Icon: Trash2,
          tone: "destructive",
          kind: "perform",
          perform: () => dialog.open(<DeleteMachineDialog machine={machine} />),
        }
      : undefined,
  } satisfies Record<string, Action | undefined>;
}

export function useMachinesActions() {
  const permissions = usePermissions();

  return {
    create: permissions.has("machine:write")
      ? {
          label: m.machines_action_new(),
          Icon: Plus,
          kind: "link",
          link: linkOptions({ to: "/machines/new" }),
        }
      : undefined,
  } satisfies Record<string, Action | undefined>;
}
