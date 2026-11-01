import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useStartRecovery } from "@/features/users/lib/api";
import type {
  RecoveryLinkResponse,
  UserResponse,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function StartRecoveryDialog({
  user,
  onStarted,
}: {
  user: UserResponse;
  onStarted: (link: RecoveryLinkResponse) => void;
}) {
  const startRecovery = useStartRecovery();

  return (
    <ConfirmationDialog
      title={m.user_start_recovery_title({ username: user.username })}
      description={m.user_start_recovery_description({
        username: user.username,
      })}
      confirmLabel={m.user_start_recovery_confirm()}
      tone="destructive"
      errorTitle={m.user_start_recovery_error()}
      onConfirm={async () => {
        onStarted(await startRecovery.mutateAsync({ path: { id: user.id } }));
      }}
    />
  );
}
