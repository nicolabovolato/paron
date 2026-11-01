import { Pencil } from "lucide-react";
import { useState } from "react";
import { z } from "zod";

import { ActionButton } from "@/components/actions";
import { CardAction, CardHeader } from "@/components/card";
import { DetailItem } from "@/components/detail-item";
import { errorToast } from "@/components/error-toast";
import { useAppForm, validationLogic } from "@/components/form";
import { Button } from "@/components/shadcn/button";
import {
  Card,
  CardContent,
  CardFooter,
  CardTitle,
} from "@/components/shadcn/card";
import { FieldGroup } from "@/components/shadcn/field";
import { useToastManager } from "@/components/shadcn/toast";
import { useUpdateUser } from "@/features/users/lib/api";
import type { UserResponse } from "@/generated/api/types.gen";
import { zDisplayName } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

const schema = z.object({ display_name: z.string().trim().pipe(zDisplayName) });

/** The profile of the signed-in user, which they change in place. */
export function ProfileCard({ user }: { user: UserResponse }) {
  const update = useUpdateUser();
  const toast = useToastManager();
  const [editing, setEditing] = useState(false);

  const form = useAppForm({
    defaultValues: { display_name: user.display_name },
    validationLogic,
    validators: { onDynamic: schema },
    onSubmit: async ({ value }) => {
      try {
        await update.mutateAsync({
          path: { id: user.id },
          body: schema.parse(value),
        });
        toast.add({ title: m.user_display_name_saved(), type: "success" });
        setEditing(false);
      } catch (error) {
        if (!isApiError(error)) throw error;
        toast.add(errorToast(m.user_display_name_error(), error));
      }
    },
  });

  function cancel() {
    form.reset();
    setEditing(false);
  }

  return (
    <form.AppForm>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") cancel();
        }}
      >
        <Card>
          <CardHeader>
            <CardTitle>{m.user_profile_title()}</CardTitle>
            {!editing && (
              <CardAction>
                <ActionButton
                  action={{
                    label: m.user_action_edit(),
                    Icon: Pencil,
                    kind: "perform",
                    perform: () => {
                      form.reset({ display_name: user.display_name });
                      setEditing(true);
                      return true;
                    },
                  }}
                />
              </CardAction>
            )}
          </CardHeader>
          <CardContent>
            {editing ? (
              <FieldGroup>
                <form.AppField name="display_name">
                  {(field) => (
                    <field.TextField
                      label={m.user_display_name_label()}
                      placeholder={m.user_display_name_placeholder()}
                      autoFocus
                    />
                  )}
                </form.AppField>
              </FieldGroup>
            ) : (
              <dl className="grid grid-cols-1 gap-4 text-sm sm:grid-cols-2">
                <DetailItem term={m.user_display_name_label()}>
                  {user.display_name}
                </DetailItem>
              </dl>
            )}
          </CardContent>
          {editing && (
            <CardFooter className="justify-end gap-2">
              <Button type="button" variant="ghost" onClick={cancel}>
                {m.form_cancel()}
              </Button>
              <form.SubmitButton>{m.user_save_changes()}</form.SubmitButton>
            </CardFooter>
          )}
        </Card>
      </form>
    </form.AppForm>
  );
}
