import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useToastManager } from "@/components/shadcn/toast";
import { useWithdrawMachineEnrollment } from "@/features/machines/lib/api";
import type { MachineResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function RevokeTunnelAccessDialog({
  machine,
}: {
  machine: MachineResponse;
}) {
  const withdraw = useWithdrawMachineEnrollment();
  const toast = useToastManager();

  return (
    <ConfirmationDialog
      title={m.tunnel_revoke_title({ name: machine.name })}
      description={m.tunnel_revoke_description({ name: machine.name })}
      confirmLabel={m.tunnel_revoke_confirm()}
      tone="destructive"
      errorTitle={m.tunnel_revoke_error()}
      onConfirm={async () => {
        await withdraw.mutateAsync({ path: { id: machine.id } });
        toast.add({
          title: m.tunnel_revoke_success(),
          description: machine.name,
          type: "success",
        });
      }}
    />
  );
}
