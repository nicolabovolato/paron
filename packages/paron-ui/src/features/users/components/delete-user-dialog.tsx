import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useToastManager } from "@/components/shadcn/toast";
import { useRemoveUser } from "@/features/users/lib/api";
import type { UserResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function DeleteUserDialog({ user }: { user: UserResponse }) {
  const remove = useRemoveUser();
  const toast = useToastManager();

  return (
    <ConfirmationDialog
      title={m.user_delete_title({ username: user.username })}
      description={m.user_delete_description()}
      confirmLabel={m.user_delete_confirm()}
      tone="destructive"
      typeToConfirm={user.username}
      errorTitle={m.user_delete_error()}
      onConfirm={async () => {
        await remove.mutateAsync({ path: { id: user.id } });
        toast.add({
          type: "success",
          title: m.user_delete_success(),
          description: user.username,
        });
      }}
    />
  );
}
