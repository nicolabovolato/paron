import { withFieldGroup } from "@/components/form";
import { zPassword } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";

export const NewPasswordFields = withFieldGroup({
  defaultValues: { password: "", confirm_password: "" },
  render: function NewPasswordFieldsRender({ group }) {
    return (
      <>
        <group.AppField name="password" validators={{ onDynamic: zPassword }}>
          {(field) => (
            <field.TextField
              label={m.new_password_label()}
              type="password"
              autoComplete="new-password"
              description={m.new_password_length()}
              autoFocus
            />
          )}
        </group.AppField>

        <group.AppField
          name="confirm_password"
          validators={{
            onChangeListenTo: ["password"],
            onDynamic: ({ value }) =>
              value === group.getFieldValue("password")
                ? undefined
                : { message: m.new_password_mismatch() },
          }}
        >
          {(field) => (
            <field.TextField
              label={m.new_password_confirm_label()}
              type="password"
              autoComplete="new-password"
            />
          )}
        </group.AppField>
      </>
    );
  },
});
