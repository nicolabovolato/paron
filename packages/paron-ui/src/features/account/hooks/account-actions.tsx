import { useMutation } from "@tanstack/react-query";
import {
  Fingerprint,
  KeyRound,
  ListRestart,
  Smartphone,
  Trash2,
} from "lucide-react";

import type { Action } from "@/components/actions";
import { ConfirmationDialog } from "@/components/confirmation-dialog";
import { useDialog } from "@/components/dialog";
import { errorToast } from "@/components/error-toast";
import { useToastManager } from "@/components/shadcn/toast";
import {
  endedWithoutPasskey,
  passkeyErrorMessage,
  useReauthentication,
} from "@/components/sign-in";
import { RecoveryCodesDialog } from "@/features/account/components/recovery-codes-dialog";
import { SetPasswordDialog } from "@/features/account/components/set-password-dialog";
import { TotpSetupDialog } from "@/features/account/components/totp-setup-dialog";
import {
  useStartCredentialAction,
  useUpdateCredentials,
} from "@/features/account/lib/api";
import { updateCredentials as sendCredentialUpdate } from "@/generated/api/sdk.gen";
import type {
  CredentialsResponse,
  PasskeyCredentialResponse,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";
import { createPasskey, passkeysSupported } from "@/lib/passkeys";

export function useCredentialActions(credentials: CredentialsResponse) {
  const dialog = useDialog();
  const toast = useToastManager();
  const startTotp = useStartCredentialAction();
  const updateCredentials = useUpdateCredentials();
  const withReauthentication = useReauthentication();
  const addPasskey = useMutation(
    withReauthentication({
      mutationFn: async () => {
        const { data: started } = await sendCredentialUpdate({
          body: { action: "start_passkey_registration" },
          throwOnError: true,
        });
        if (started.result.kind !== "passkey_creation_options")
          throw new Error(`Unexpected answer: ${started.result.kind}`);
        const { data: added } = await sendCredentialUpdate({
          body: {
            action: "finish_passkey_registration",
            response: await createPasskey(started.result.options),
          },
          throwOnError: true,
        });
        return added;
      },
      retry: false,
    }),
  );

  async function showRecoveryCodes(codes: string[] | null | undefined) {
    if (codes) await dialog.open(<RecoveryCodesDialog codes={codes} />);
  }

  const hasPassword = Boolean(credentials.password);
  const hasTotp = Boolean(credentials.totp);

  return {
    setPassword: {
      label: hasPassword
        ? m.account_action_change_password()
        : m.account_action_set_password(),
      Icon: KeyRound,
      kind: "perform",
      perform: () => dialog.open(<SetPasswordDialog replaces={hasPassword} />),
    },

    removePassword: hasPassword
      ? {
          label: m.account_action_remove_password(),
          Icon: Trash2,
          tone: "destructive",
          kind: "perform",
          perform: () =>
            dialog.open(
              <ConfirmationDialog
                title={m.password_remove_title()}
                confirmLabel={m.password_remove_confirm()}
                tone="destructive"
                errorTitle={m.password_remove_error()}
                onConfirm={() =>
                  updateCredentials.mutateAsync({
                    body: { action: "remove_password" },
                  })
                }
              />,
            ),
        }
      : undefined,

    setUpTotp: hasTotp
      ? undefined
      : {
          label: m.account_action_set_up_totp(),
          Icon: Smartphone,
          pending: startTotp.isPending,
          kind: "perform",
          perform: async () => {
            let started;
            try {
              started = await startTotp.mutateAsync({
                body: { action: "start_totp_registration" },
              });
            } catch (error) {
              if (!isApiError(error)) throw error;
              toast.add(errorToast(m.totp_setup_error(), error));
              return false;
            }
            if (started.result.kind !== "totp_registration_started")
              return false;

            let enabled: { recoveryCodes?: string[] | null } | undefined;
            await dialog.open(
              <TotpSetupDialog
                setup={started.result}
                onEnabled={(recoveryCodes) => (enabled = { recoveryCodes })}
              />,
            );
            if (!enabled) return false;

            toast.add({ type: "success", title: m.totp_enabled() });
            await showRecoveryCodes(enabled.recoveryCodes);
            return true;
          },
        },

    removeTotp: hasTotp
      ? {
          label: m.account_action_remove_totp(),
          Icon: Trash2,
          tone: "destructive",
          kind: "perform",
          perform: () =>
            dialog.open(
              <ConfirmationDialog
                title={m.totp_remove_title()}
                confirmLabel={m.totp_remove_confirm()}
                tone="destructive"
                errorTitle={m.totp_remove_error()}
                onConfirm={() =>
                  updateCredentials.mutateAsync({
                    body: { action: "remove_totp" },
                  })
                }
              />,
            ),
        }
      : undefined,

    addPasskey: {
      label: m.account_action_add_passkey(),
      Icon: Fingerprint,
      disabled: !passkeysSupported() && {
        reason: m.passkey_blocked_by_browser(),
      },
      pending: addPasskey.isPending,
      kind: "perform",
      perform: async () => {
        let added;
        try {
          added = await addPasskey.mutateAsync();
        } catch (error) {
          if (endedWithoutPasskey(error)) return false;
          const passkeyError = passkeyErrorMessage(error);
          if (passkeyError) {
            toast.add({
              type: "error",
              title: m.passkey_add_error(),
              description: passkeyError,
            });
            return false;
          }
          if (!isApiError(error)) throw error;
          toast.add(errorToast(m.passkey_add_error(), error));
          return false;
        }

        toast.add({ type: "success", title: m.passkey_added() });
        await showRecoveryCodes(added.recovery_codes);
        return true;
      },
    },

    generateRecoveryCodes: credentials.recovery_codes
      ? {
          label: m.account_action_new_recovery_codes(),
          Icon: ListRestart,
          tone: "warning",
          kind: "perform",
          perform: async () => {
            let codes: string[] | undefined;
            await dialog.open(
              <ConfirmationDialog
                title={m.recovery_codes_new_title()}
                description={m.recovery_codes_new_description()}
                confirmLabel={m.recovery_codes_new_confirm()}
                tone="warning"
                errorTitle={m.recovery_codes_new_error()}
                onConfirm={async () => {
                  const generated = await updateCredentials.mutateAsync({
                    body: { action: "generate_recovery_codes" },
                  });
                  codes = generated.recovery_codes ?? undefined;
                }}
              />,
            );
            if (!codes) return false;

            await showRecoveryCodes(codes);
            return true;
          },
        }
      : undefined,
  } satisfies Record<string, Action | undefined>;
}

export function usePasskeyActions(passkey: PasskeyCredentialResponse) {
  const dialog = useDialog();
  const updateCredentials = useUpdateCredentials();

  return {
    remove: {
      label: m.passkey_action_remove({ name: passkey.name }),
      Icon: Trash2,
      tone: "destructive",
      kind: "perform",
      perform: () =>
        dialog.open(
          <ConfirmationDialog
            title={m.passkey_remove_title({ name: passkey.name })}
            confirmLabel={m.passkey_remove_confirm()}
            tone="destructive"
            errorTitle={m.passkey_remove_error()}
            onConfirm={() =>
              updateCredentials.mutateAsync({
                body: { action: "remove_passkey", id: passkey.id },
              })
            }
          />,
        ),
    },
  } satisfies Record<string, Action | undefined>;
}
