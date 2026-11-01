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
import { useToastManager } from "@/components/shadcn/toast";
import { NewPasswordFields } from "@/features/account/components/new-password-fields";
import { useUpdateCredentials } from "@/features/account/lib/api";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

/** `replaces` when the user has a password: the dialog changes it. Else it adds one. */
export function SetPasswordDialog({ replaces }: { replaces: boolean }) {
  const dialog = useDialog();
  const updateCredentials = useUpdateCredentials();
  const toast = useToastManager();

  const form = useAppForm({
    defaultValues: {
      new_password: "",
      confirm_password: "",
    },
    validationLogic,
    validators: {
      onSubmitAsync: async ({ value, formApi }) => {
        try {
          await updateCredentials.mutateAsync({
            body: { action: "set_password", new_password: value.new_password },
          });
          toast.add({
            title: replaces
              ? m.account_change_password_success()
              : m.account_set_password_success(),
            description: m.account_change_password_success_description(),
            type: "success",
          });
          dialog.close();
        } catch (error) {
          if (!isApiError(error)) throw error;
          return toFormValidationError(formApi, error);
        }
      },
    },
  });

  return (
    <DialogContent className="sm:max-w-100">
      <form.AppForm>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void form.handleSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>
              {replaces
                ? m.account_change_password_title()
                : m.account_set_password_title()}
            </DialogTitle>
            <DialogDescription>
              {m.account_change_password_description()}
            </DialogDescription>
          </DialogHeader>

          <form.SubmitError
            title={
              replaces
                ? m.account_change_password_error()
                : m.account_set_password_error()
            }
          />

          <NewPasswordFields
            form={form}
            fields={{
              password: "new_password",
              confirm_password: "confirm_password",
            }}
          />

          <DialogFooter>
            <form.SubmitButton>
              {replaces
                ? m.account_change_password_submit()
                : m.account_set_password_submit()}
            </form.SubmitButton>
          </DialogFooter>
        </form>
      </form.AppForm>
    </DialogContent>
  );
}
