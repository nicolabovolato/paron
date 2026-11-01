import { useId } from "react";

import { withFieldGroup } from "@/components/form";
import { FieldSet } from "@/components/shadcn/field";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/shadcn/tabs";
import type { MachineResponse } from "@/generated/api/types.gen";
import { zProtocol } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { PROTOCOL_NAMES } from "@/lib/protocols";

import { AuthenticationSection } from "./authentication-section";
import { initialValues } from "./schema";

export const ProtocolSection = withFieldGroup({
  defaultValues: initialValues().protocols,
  props: { machine: undefined as MachineResponse | undefined },
  render: function ProtocolSectionFields({ group, machine }) {
    const titleId = useId();

    return (
      <Tabs defaultValue={zProtocol.enum.ssh}>
        <FieldSet aria-labelledby={titleId}>
          <div className="flex items-center gap-3">
            <div id={titleId} className="text-base font-medium">
              {m.machine_field_protocol()}
            </div>
            <TabsList aria-labelledby={titleId}>
              <TabsTrigger value={zProtocol.enum.ssh}>
                {PROTOCOL_NAMES.ssh}
              </TabsTrigger>
            </TabsList>
          </div>
          <TabsContent
            value={zProtocol.enum.ssh}
            className="flex flex-col gap-4"
          >
            <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
              <group.AppField name="ssh.user">
                {(field) => (
                  <field.TextField
                    label={m.machine_field_user()}
                    placeholder={m.machine_form_user_placeholder()}
                    className="font-mono"
                  />
                )}
              </group.AppField>
              <group.AppField name="ssh.port">
                {(field) => (
                  <field.NumberField
                    label={m.machine_field_port()}
                    placeholder="22"
                    className="font-mono"
                  />
                )}
              </group.AppField>
              <group.AppField name="ssh.connect_timeout_seconds">
                {(field) => (
                  <field.NumberField
                    label={m.machine_form_timeout_label()}
                    placeholder="10"
                    className="font-mono"
                  />
                )}
              </group.AppField>
            </div>
            <group.AppField name="ssh.authentication.host_key">
              {(field) => (
                <field.TextareaField
                  label={m.machine_field_host_key()}
                  description={m.machine_form_host_key_description()}
                  className="font-mono text-xs"
                  // eslint-disable-next-line i18next/no-literal-string -- an example key, not language
                  placeholder="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI..."
                />
              )}
            </group.AppField>

            <AuthenticationSection
              form={group}
              fields="ssh.authentication.method"
              saved={machine?.protocols.ssh?.authentication.method}
            />
          </TabsContent>
        </FieldSet>
      </Tabs>
    );
  },
});
