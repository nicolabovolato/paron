import { z } from "zod";

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
import { RolesField } from "@/features/users/components/roles";
import { useCreateUser } from "@/features/users/lib/api";
import type {
  CreateUserRequest,
  Role,
  UserResponse,
} from "@/generated/api/types.gen";
import {
  zCreateUserRequest,
  zDisplayName,
  zUsername,
} from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

const schema = zCreateUserRequest.extend({
  username: z.string().trim().pipe(zUsername),
  display_name: z.string().trim().pipe(zDisplayName),
});

export function AddUserDialog({
  onCreated,
}: {
  onCreated: (user: UserResponse) => void;
}) {
  const dialog = useDialog();
  const create = useCreateUser();

  const form = useAppForm({
    defaultValues: {
      username: "",
      display_name: "",
      roles: [] as Role[],
    } satisfies CreateUserRequest,
    validationLogic,
    validators: {
      onDynamic: schema,
      onSubmitAsync: async ({ value, formApi }) => {
        try {
          onCreated(await create.mutateAsync({ body: schema.parse(value) }));
          dialog.close();
        } catch (error) {
          if (!isApiError(error)) throw error;
          return toFormValidationError(formApi, error);
        }
      },
    },
  });

  return (
    <DialogContent className="sm:max-w-110">
      <form.AppForm>
        <form
          className="flex flex-col gap-4"
          onSubmit={(event) => {
            event.preventDefault();
            void form.handleSubmit();
          }}
        >
          <DialogHeader>
            <DialogTitle>{m.user_add_title()}</DialogTitle>
            <DialogDescription>{m.user_add_description()}</DialogDescription>
          </DialogHeader>

          <form.SubmitError title={m.user_create_error()} />

          <form.AppField name="username">
            {(field) => (
              <field.TextField
                label={m.user_username_label()}
                placeholder={m.user_username_placeholder()}
                description={zUsername.description}
              />
            )}
          </form.AppField>

          <form.AppField name="display_name">
            {(field) => (
              <field.TextField
                label={m.user_display_name_label()}
                placeholder={m.user_display_name_placeholder()}
              />
            )}
          </form.AppField>

          <form.Field name="roles">
            {(field) => (
              <RolesField
                value={field.state.value}
                onChange={field.handleChange}
              />
            )}
          </form.Field>

          <DialogFooter>
            <form.SubmitButton>{m.user_create_submit()}</form.SubmitButton>
          </DialogFooter>
        </form>
      </form.AppForm>
    </DialogContent>
  );
}
