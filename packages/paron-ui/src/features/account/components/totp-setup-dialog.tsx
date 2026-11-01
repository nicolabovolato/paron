import { REGEXP_ONLY_DIGITS } from "input-otp";
import { z } from "zod";

import { CodeBlock } from "@/components/code-block";
import { DialogContent, useDialog } from "@/components/dialog";
import {
  toFormValidationError,
  useAppForm,
  validationLogic,
} from "@/components/form";
import {
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/shadcn/dialog";
import { QrCode } from "@/features/account/components/qr-code";
import { useUpdateCredentials } from "@/features/account/lib/api";
import type { CredentialUpdateResultResponse } from "@/generated/api/types.gen";
import { zTotpCode } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

const schema = z.object({ code: zTotpCode });

export function TotpSetupDialog({
  setup,
  onEnabled,
}: {
  setup: Extract<
    CredentialUpdateResultResponse,
    { kind: "totp_registration_started" }
  >;
  onEnabled: (recoveryCodes: string[] | null | undefined) => void;
}) {
  const dialog = useDialog();
  const updateCredentials = useUpdateCredentials();

  const form = useAppForm({
    defaultValues: { code: "" },
    validationLogic,
    validators: {
      onDynamic: schema,
      onSubmitAsync: async ({ value, formApi }) => {
        try {
          const confirmed = await updateCredentials.mutateAsync({
            body: { action: "finish_totp_registration", code: value.code },
          });
          onEnabled(confirmed.recovery_codes);
          dialog.close();
        } catch (error) {
          if (!isApiError(error)) throw error;
          if (error.status === 400)
            return { fields: { code: { message: m.totp_setup_code_wrong() } } };
          return toFormValidationError(formApi, error);
        }
      },
    },
  });

  return (
    <DialogContent className="sm:max-w-md">
      <form.AppForm>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void form.handleSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{m.totp_setup_title()}</DialogTitle>
            <DialogDescription>{m.totp_setup_description()}</DialogDescription>
          </DialogHeader>

          <div className="flex justify-center">
            <QrCode value={setup.key_uri} label={m.totp_setup_qr_label()} />
          </div>

          <div className="flex flex-col gap-2">
            <p className="text-sm text-muted-foreground">
              {m.totp_setup_secret()}
            </p>
            <CodeBlock value={setup.secret} copyable />
          </div>

          <form.AppField name="code">
            {(field) => (
              <field.CodeField
                centered
                label={m.second_factor_code_label()}
                groups={[3, 3]}
                pattern={REGEXP_ONLY_DIGITS}
                inputMode="numeric"
                autoComplete="one-time-code"
                autoFocus
              />
            )}
          </form.AppField>

          <form.SubmitError title={m.totp_setup_error()} />

          <DialogFooter>
            <form.SubmitButton>{m.totp_setup_submit()}</form.SubmitButton>
          </DialogFooter>
        </form>
      </form.AppForm>
    </DialogContent>
  );
}
