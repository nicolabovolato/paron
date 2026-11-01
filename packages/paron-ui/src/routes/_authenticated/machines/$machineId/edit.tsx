import { useSuspenseQuery } from "@tanstack/react-query";
import {
  createFileRoute,
  linkOptions,
  useNavigate,
} from "@tanstack/react-router";

import { BackButton } from "@/components/route/back-button";
import { Page } from "@/components/route/page";
import { MachineForm } from "@/features/machines/components/machine-form";
import { machineQueries } from "@/features/machines/lib/api";
import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute(
  "/_authenticated/machines/$machineId/edit",
)({
  staticData: { permission: "machine:write" },
  loader: () => ({ breadcrumb: m.machine_edit_crumb() }),
  component: RouteComponent,
});

function RouteComponent() {
  const { machineId } = Route.useParams();
  const navigate = useNavigate();
  const { data: machine } = useSuspenseQuery(machineQueries.detail(machineId));

  return (
    <Page title={m.machine_form_edit_title({ name: machine.name })}>
      <MachineForm
        machine={machine}
        cancel={
          <BackButton
            label={m.form_cancel()}
            variant="ghost"
            fallback={linkOptions({
              to: "/machines/$machineId",
              params: { machineId },
            })}
          />
        }
        onSaved={() =>
          void navigate({ to: "/machines/$machineId", params: { machineId } })
        }
      />
    </Page>
  );
}
