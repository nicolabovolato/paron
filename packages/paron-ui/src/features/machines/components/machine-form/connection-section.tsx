import { withFieldGroup } from "@/components/form";
import { FieldLegend, FieldSet } from "@/components/shadcn/field";
import { RadioGroup } from "@/components/shadcn/radio-group";
import type { MachineResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

import { ChoiceCard } from "@/components/choice-card";
import { initialValues, type MachineFormValues } from "./schema";
import { Warning } from "./warning";

export const ConnectionSection = withFieldGroup({
  defaultValues: initialValues().connectivity,
  props: { machine: undefined as MachineResponse | undefined },
  render: function ConnectionSectionFields({ group, machine }) {
    return (
      <FieldSet>
        <FieldLegend>{m.machine_field_connection()}</FieldLegend>
        <group.Field name="kind">
          {(field) => (
            <RadioGroup
              value={field.state.value}
              onValueChange={(value) =>
                field.handleChange(
                  value as MachineFormValues["connectivity"]["kind"],
                )
              }
              className="grid-cols-1 md:grid-cols-2"
            >
              <ChoiceCard
                value="direct"
                title={m.machine_connectivity_direct()}
                description={m.machine_form_connection_direct_description()}
              />
              <ChoiceCard
                value="tunneled"
                title={m.machine_connectivity_tunneled()}
                description={m.machine_form_connection_tunneled_description()}
              />
            </RadioGroup>
          )}
        </group.Field>

        <group.Subscribe selector={(state) => state.values.kind}>
          {(kind) =>
            kind === "direct" && (
              <group.AppField name="host">
                {(field) => (
                  <field.TextField
                    label={m.machine_field_host()}
                    // eslint-disable-next-line i18next/no-literal-string -- example addresses, not language
                    placeholder="10.0.0.1 / machine.example.com"
                    className="font-mono"
                  />
                )}
              </group.AppField>
            )
          }
        </group.Subscribe>

        <group.Subscribe
          selector={(state) => connectivityWarning(state.values, machine)}
        >
          {(warning) => warning && <Warning {...warning} />}
        </group.Subscribe>
      </FieldSet>
    );
  },
});

function connectivityWarning(
  connectivity: MachineFormValues["connectivity"],
  machine?: MachineResponse,
) {
  if (
    machine?.connectivity.kind === "direct" &&
    connectivity.kind === "tunneled"
  ) {
    return {
      title: m.machine_form_to_tunneled_title(),
      description: m.machine_form_to_tunneled_description(),
    };
  }
  if (
    machine?.connectivity.kind !== "tunneled" ||
    connectivity.kind !== "direct"
  )
    return undefined;

  switch (machine.connectivity.enrollment.state) {
    case "enrolled":
      return {
        title: m.machine_form_revoke_title(),
        description: m.tunnel_revoke_description({ name: machine.name }),
      };
    case "invited":
      return {
        title: m.machine_form_discard_setup_title(),
        description: m.machine_form_discard_setup_description(),
      };
    case "not_enrolled":
      return undefined;
  }
}
