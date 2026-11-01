import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useToastManager } from "@/components/shadcn/toast";
import { useRemoveMachine } from "@/features/machines/lib/api";
import type { MachineResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function DeleteMachineDialog({ machine }: { machine: MachineResponse }) {
  const remove = useRemoveMachine();
  const toast = useToastManager();

  return (
    <ConfirmationDialog
      title={m.machine_delete_title()}
      description={m.machine_delete_description()}
      confirmLabel={m.machine_delete_confirm()}
      tone="destructive"
      typeToConfirm={machine.name}
      errorTitle={m.machine_delete_error()}
      onConfirm={async () => {
        await remove.mutateAsync({ path: { id: machine.id } });
        toast.add({
          title: m.machine_delete_success(),
          description: machine.name,
          type: "success",
        });
      }}
    />
  );
}
