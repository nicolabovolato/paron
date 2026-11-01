import { useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute, useNavigate } from "@tanstack/react-router";

import { ActionButton } from "@/components/actions";
import { CardAction, CardHeader } from "@/components/card";
import { CodeBlock } from "@/components/code-block";
import { DetailItem } from "@/components/detail-item";
import { Page } from "@/components/route/page";
import { Card, CardContent, CardTitle } from "@/components/shadcn/card";
import {
  MachineAuthenticationBadge,
  MachineEnrollmentBadge,
} from "@/features/machines/components/machine-badges";
import { useMachineActions } from "@/features/machines/hooks/machine-actions";
import { machineQueries } from "@/features/machines/lib/api";
import { m } from "@/generated/paraglide/messages";
import { MachineTabs } from "@/routes/_authenticated/machines/$machineId/-tabs";

export const Route = createFileRoute("/_authenticated/machines/$machineId/")({
  staticData: { width: "narrow" },
  component: RouteComponent,
});

function RouteComponent() {
  const { machineId } = Route.useParams();
  const { data: machine } = useSuspenseQuery(machineQueries.detail(machineId));
  const actions = useMachineActions(machine);
  const navigate = useNavigate();
  const connection = [
    actions.testConnection,
    actions.setUpTunnel,
    actions.cancelTunnelSetup,
    actions.revokeTunnelAccess,
  ].filter((action) => action !== undefined);
  const { connectivity } = machine;
  const { ssh } = machine.protocols;

  return (
    <Page
      title={machine.name}
      actions={
        <div className="flex gap-2">
          {actions.edit && <ActionButton action={actions.edit} />}
          {actions.openTerminal && (
            <ActionButton action={actions.openTerminal} variant="default" />
          )}
        </div>
      }
    >
      <MachineTabs />
      <Card>
        <CardHeader>
          <CardTitle>{m.machine_field_connection()}</CardTitle>
          {connection.length > 0 && (
            <CardAction>
              {connection.map((action) => (
                <ActionButton key={action.label} action={action} />
              ))}
            </CardAction>
          )}
        </CardHeader>
        <CardContent>
          <dl className="grid grid-cols-1 gap-4 text-sm sm:grid-cols-4">
            <DetailItem term={m.machine_detail_type()}>
              {connectivity.kind === "direct"
                ? m.machine_connectivity_direct()
                : m.machine_connectivity_tunneled()}
            </DetailItem>
            {connectivity.kind === "direct" ? (
              <DetailItem
                term={m.machine_field_host()}
                className="sm:col-span-3"
                valueClassName="font-mono"
              >
                {connectivity.host}
              </DetailItem>
            ) : (
              <DetailItem
                term={m.machine_detail_tunnel()}
                className="sm:col-span-3"
                valueClassName="flex flex-wrap items-center gap-x-2 gap-y-1"
              >
                {connectivity.enrollment.state === "enrolled" ? (
                  m.tunnel_state_registered()
                ) : (
                  <MachineEnrollmentBadge
                    enrollment={connectivity.enrollment}
                  />
                )}
              </DetailItem>
            )}
            {connectivity.kind === "tunneled" &&
              connectivity.enrollment.state === "enrolled" && (
                <DetailItem
                  term={m.machine_detail_agent_public_key()}
                  className="sm:col-span-4"
                  valueClassName="min-w-0 font-normal"
                >
                  <CodeBlock
                    value={connectivity.enrollment.public_key}
                    copyable
                  />
                </DetailItem>
              )}
          </dl>
        </CardContent>
      </Card>

      {ssh && (
        <Card>
          <CardHeader>
            <CardTitle>SSH</CardTitle>
          </CardHeader>
          <CardContent>
            <dl className="grid grid-cols-1 gap-4 text-sm sm:grid-cols-4">
              <DetailItem
                term={m.machine_field_user()}
                valueClassName="font-mono"
              >
                {ssh.user}
              </DetailItem>
              <DetailItem
                term={m.machine_field_port()}
                valueClassName="font-mono"
              >
                {ssh.port}
              </DetailItem>
              <DetailItem term={m.machine_detail_connect_timeout()}>
                {m.machine_detail_connect_timeout_value({
                  seconds: ssh.connect_timeout_seconds,
                })}
              </DetailItem>
              <DetailItem term={m.machine_field_authentication()}>
                <MachineAuthenticationBadge
                  authentication={ssh.authentication.method}
                />
              </DetailItem>
              <DetailItem
                term={m.machine_field_host_key()}
                className="sm:col-span-4"
                valueClassName="min-w-0 font-normal"
              >
                <CodeBlock value={ssh.authentication.host_key} copyable />
              </DetailItem>
              {ssh.authentication.method.kind === "keypair" && (
                <DetailItem
                  term={m.machine_detail_public_key()}
                  className="sm:col-span-4"
                  valueClassName="min-w-0 font-normal"
                >
                  <CodeBlock
                    value={ssh.authentication.method.public_key}
                    copyable
                  />
                </DetailItem>
              )}
            </dl>
          </CardContent>
        </Card>
      )}

      {actions.delete && (
        <Card className="ring-destructive/40">
          <CardHeader>
            <CardTitle>{m.machine_detail_delete_title()}</CardTitle>
            <CardAction>
              <ActionButton
                action={actions.delete}
                onPerformed={() => void navigate({ to: "/machines" })}
              />
            </CardAction>
          </CardHeader>
        </Card>
      )}
    </Page>
  );
}
