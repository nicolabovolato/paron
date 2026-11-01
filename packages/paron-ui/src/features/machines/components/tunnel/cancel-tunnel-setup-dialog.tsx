import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useToastManager } from "@/components/shadcn/toast";
import { useWithdrawMachineEnrollment } from "@/features/machines/lib/api";
import type { MachineResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function CancelTunnelSetupDialog({
  machine,
}: {
  machine: MachineResponse;
}) {
  const withdraw = useWithdrawMachineEnrollment();
  const toast = useToastManager();

  return (
    <ConfirmationDialog
      title={m.tunnel_cancel_title()}
      description={m.tunnel_cancel_description({ name: machine.name })}
      confirmLabel={m.tunnel_cancel_confirm()}
      tone="warning"
      errorTitle={m.tunnel_cancel_error()}
      onConfirm={async () => {
        await withdraw.mutateAsync({ path: { id: machine.id } });
        toast.add({
          title: m.tunnel_cancel_success(),
          description: machine.name,
          type: "success",
        });
      }}
    />
  );
}
