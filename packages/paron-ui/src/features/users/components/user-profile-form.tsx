import { useState, type ReactNode } from "react";
import { z } from "zod";

import { errorToast } from "@/components/error-toast";
import {
  toFormValidationError,
  useAppForm,
  validationLogic,
} from "@/components/form";
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/shadcn/card";
import { FieldGroup } from "@/components/shadcn/field";
import { useToastManager } from "@/components/shadcn/toast";
import { RolesField } from "@/features/users/components/roles";
import { useUpdateUser } from "@/features/users/lib/api";
import type {
  PartialUpdateUserRequest,
  UserResponse,
} from "@/generated/api/types.gen";
import { zDisplayName, zRole, zUsername } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

const schema = z.object({
  username: z.string().trim().pipe(zUsername),
  display_name: z.string().trim().pipe(zDisplayName),
  roles: z.array(zRole),
});

type Profile = z.infer<typeof schema>;

export function UserProfileForm({
  user,
  onSaved,
  cancel,
}: {
  user: UserResponse;
  onSaved?: () => void;
  cancel?: ReactNode;
}) {
  const update = useUpdateUser();
  const toast = useToastManager();
  // Refetches do not update this value. A save compares the form with it and
  // sends only the changed fields.
  const [editedUser] = useState(user);

  const form = useAppForm({
    defaultValues: {
      username: editedUser.username,
      display_name: editedUser.display_name,
      roles: editedUser.roles,
    } satisfies Profile,
    validationLogic,
    validators: {
      onDynamic: schema,
      onSubmitAsync: async ({ value, formApi }) => {
        const profile = schema.parse(value);
        try {
          const updated = await update.mutateAsync({
            path: { id: editedUser.id },
            body: {
              username:
                profile.username !== editedUser.username
                  ? profile.username
                  : undefined,
              display_name:
                profile.display_name !== editedUser.display_name
                  ? profile.display_name
                  : undefined,
              roles:
                [...profile.roles].sort().join() !==
                [...editedUser.roles].sort().join()
                  ? profile.roles
                  : undefined,
            } satisfies PartialUpdateUserRequest,
          });

          toast.add({
            title: m.user_update_success(),
            description: updated.username,
            type: "success",
          });
          onSaved?.();
        } catch (error) {
          if (!isApiError(error)) throw error;
          const result = toFormValidationError(formApi, error);
          if (result.form)
            toast.add(errorToast(m.user_update_error(), result.form));
          return { fields: result.fields };
        }
      },
    },
  });

  return (
    <form.AppForm>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
      >
        <Card>
          <CardHeader>
            <CardTitle>{m.user_profile_title()}</CardTitle>
          </CardHeader>
          <CardContent>
            <FieldGroup>
              <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                <form.AppField name="username">
                  {(field) => (
                    <field.TextField
                      label={m.user_username_label()}
                      placeholder={m.user_username_placeholder()}
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
              </div>
              <form.Field name="roles">
                {(field) => (
                  <RolesField
                    value={field.state.value}
                    onChange={field.handleChange}
                  />
                )}
              </form.Field>
            </FieldGroup>
          </CardContent>
          <CardFooter className="justify-end gap-2">
            {cancel}
            <form.SubmitButton>{m.user_save_changes()}</form.SubmitButton>
          </CardFooter>
        </Card>
      </form>
    </form.AppForm>
  );
}
