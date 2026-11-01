import { useSelector, type AnyFormApi } from "@tanstack/react-form";
import { PlugZap } from "lucide-react";
import { useState, type ReactNode } from "react";

import { ActionButton, type Action } from "@/components/actions";
import { ErrorAlert } from "@/components/error-alert";
import {
  toFormValidationError,
  useAppForm,
  validationLogic,
} from "@/components/form";
import { Card, CardContent, CardFooter } from "@/components/shadcn/card";
import { FieldGroup, FieldSeparator } from "@/components/shadcn/field";
import { useToastManager } from "@/components/shadcn/toast";
import { useConnectionTest } from "@/features/machines/hooks/connection-test";
import {
  useCreateMachine,
  useUpdateMachine,
} from "@/features/machines/lib/api";
import { testConnectionDisabled } from "@/features/machines/lib/utils";
import type { MachineResponse } from "@/generated/api/types.gen";
import { zProtocol } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { usePermissions } from "@/hooks/permissions";
import { useUnsavedChangesGuard } from "@/hooks/unsaved-changes-guard";
import { isApiError, type ApiError } from "@/lib/api-error";

import { ConnectionSection } from "./connection-section";
import { ProtocolSection } from "./protocol-section";
import {
  initialValues,
  machineFormSchema,
  toCreateRequest,
  toUpdateRequest,
} from "./schema";

// Puts the validation messages of a failed request on their fields, and gives
// back the error when part of it has no field.
function showFieldErrors(
  form: Pick<AnyFormApi, "getFieldMeta" | "setErrorMap">,
  error: ApiError,
) {
  const { form: formError, fields } = toFormValidationError(form, error);
  form.setErrorMap({
    onDynamic: {
      fields: Object.fromEntries(
        Object.entries(fields).map(([path, fieldError]) => [
          path,
          [fieldError],
        ]),
      ),
    },
  });
  return formError;
}

export function MachineForm({
  machine,
  onSaved,
  cancel,
}: {
  machine?: MachineResponse;
  onSaved?: (machine: MachineResponse) => void;
  cancel?: ReactNode;
}) {
  const permissions = usePermissions();
  const create = useCreateMachine();
  const toast = useToastManager();
  const connectionTest = useConnectionTest(machine?.id);
  const update = useUpdateMachine();
  // Refetches do not update this value. A save compares the form with it and
  // sends only the changed fields.
  const [editedMachine] = useState(machine);

  // A form-wide error of the last save. It is not a validation error, so it
  // does not stop the next submit.
  const [saveError, setSaveError] = useState<ApiError>();
  // The form has one "submitting" state, so a test that starts during a save
  // would end it for the save too.
  const [saving, setSaving] = useState(false);

  const form = useAppForm({
    defaultValues: initialValues(editedMachine),
    validationLogic,
    validators: { onDynamic: machineFormSchema },
    // Save and Test both check the whole form first, so both are submits. The
    // test itself runs after its submit, so a save does not wait for it.
    onSubmitMeta: { test: false },
    onSubmit: async ({ value, formApi, meta }) => {
      if (meta.test) return;

      setSaveError(undefined);
      setSaving(true);
      try {
        const saved = editedMachine
          ? await update.mutateAsync({
              path: { id: editedMachine.id },
              body: toUpdateRequest(value, editedMachine),
            })
          : await create.mutateAsync({ body: toCreateRequest(value) });

        formApi.reset(value);
        toast.add({
          title: machine ? m.machine_form_saved() : m.machine_form_created(),
          type: "success",
        });
        onSaved?.(saved);
      } catch (error) {
        if (!isApiError(error)) throw error;
        setSaveError(showFieldErrors(formApi, error));
      } finally {
        setSaving(false);
      }
    },
  });

  useUnsavedChangesGuard(form);

  const testDisabled = useSelector(
    form.store,
    ({ values }) =>
      (saving && { reason: m.machine_form_test_while_saving() }) ||
      testConnectionDisabled({
        tunneled: values.connectivity.kind === "tunneled",
        enrolled:
          machine?.connectivity.kind === "tunneled" &&
          machine.connectivity.enrollment.state === "enrolled",
      }) ||
      (values.protocols.ssh.authentication.method.replace &&
      values.protocols.ssh.authentication.method.kind === "keypair" &&
      values.protocols.ssh.authentication.method.source.kind === "managed"
        ? {
            reason: m.machine_form_test_after_save(),
          }
        : false),
  );

  const testConnection = permissions.has("machine:operate")
    ? ({
        label: m.connection_test_action(),
        Icon: PlugZap,
        disabled: testDisabled,
        pending: connectionTest.pending,
        kind: "perform",
        perform: async () => {
          await form.handleSubmit({ test: true });
          if (!form.state.isSubmitSuccessful) return false;

          const values = form.state.values;
          const result = await (editedMachine
            ? connectionTest.saved(values.name, {
                path: { id: editedMachine.id },
                query: { protocol: zProtocol.enum.ssh },
                body: toUpdateRequest(values, editedMachine),
              })
            : connectionTest.unsaved(values.name, {
                query: { protocol: zProtocol.enum.ssh },
                body: toCreateRequest(values),
              }));
          if (result.ok) return true;

          showFieldErrors(form, result.error);
          return false;
        },
      } satisfies Action)
    : undefined;

  return (
    <form.AppForm>
      <form
        className="flex flex-col gap-6"
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
      >
        {saveError && (
          <ErrorAlert title={m.machine_form_save_error()} error={saveError} />
        )}

        <Card>
          <CardContent>
            <FieldGroup>
              <form.AppField name="name">
                {(field) => (
                  <field.TextField
                    label={m.machine_field_name()}
                    placeholder={m.machine_form_name_placeholder()}
                  />
                )}
              </form.AppField>

              <FieldSeparator />

              <ConnectionSection
                form={form}
                fields="connectivity"
                machine={machine}
              />

              <FieldSeparator />

              <ProtocolSection
                form={form}
                fields="protocols"
                machine={machine}
              />
            </FieldGroup>
          </CardContent>

          <CardFooter className="flex-wrap gap-3 sm:gap-2">
            {testConnection && (
              <div className="flex basis-full flex-col sm:block sm:basis-auto">
                <ActionButton action={testConnection} />
              </div>
            )}
            <div className="flex flex-1 gap-2 sm:ml-auto sm:flex-none">
              {cancel}
              <form.SubmitButton className="flex-1 sm:flex-none">
                {machine ? m.machine_form_save() : m.machine_form_create()}
              </form.SubmitButton>
            </div>
          </CardFooter>
        </Card>
      </form>
    </form.AppForm>
  );
}
