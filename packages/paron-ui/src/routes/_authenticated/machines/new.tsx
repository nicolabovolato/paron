import {
  createFileRoute,
  linkOptions,
  useNavigate,
} from "@tanstack/react-router";

import { Page } from "@/components/route/page";
import { BackButton } from "@/components/route/back-button";
import { MachineForm } from "@/features/machines/components/machine-form";
import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/machines/new")({
  staticData: { permission: "machine:write" },
  loader: () => ({ breadcrumb: m.machines_new_crumb() }),
  component: RouteComponent,
});

function RouteComponent() {
  const navigate = useNavigate();

  return (
    <Page title={m.machine_form_new_title()}>
      <MachineForm
        cancel={
          <BackButton
            label={m.form_cancel()}
            variant="ghost"
            fallback={linkOptions({ to: "/machines" })}
          />
        }
        onSaved={(machine) =>
          void navigate({
            to: "/machines/$machineId",
            params: { machineId: machine.id },
          })
        }
      />
    </Page>
  );
}
