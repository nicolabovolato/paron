import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useToastManager } from "@/components/shadcn/toast";
import { useUpdateUser } from "@/features/users/lib/api";
import type { UserResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function SuspendUserDialog({ user }: { user: UserResponse }) {
  const update = useUpdateUser();
  const toast = useToastManager();

  return (
    <ConfirmationDialog
      title={m.user_suspend_title({ username: user.username })}
      description={m.user_suspend_description()}
      confirmLabel={m.user_suspend_confirm()}
      tone="warning"
      errorTitle={m.user_suspend_error()}
      onConfirm={async () => {
        await update.mutateAsync({
          path: { id: user.id },
          body: { status: "suspended" },
        });
        toast.add({
          type: "warning",
          title: m.user_suspend_success(),
          description: user.username,
        });
      }}
    />
  );
}
