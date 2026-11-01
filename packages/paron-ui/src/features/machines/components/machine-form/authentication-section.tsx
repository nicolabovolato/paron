import { KeyRound, Lock } from "lucide-react";
import { useId } from "react";

import { withFieldGroup } from "@/components/form";
import { Button } from "@/components/shadcn/button";
import { FieldSet } from "@/components/shadcn/field";
import { RadioGroup } from "@/components/shadcn/radio-group";
import {
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@/components/shadcn/tabs";
import type { SshAuthMethodResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

import { ChoiceCard } from "@/components/choice-card";
import { initialValues, type MachineFormValues } from "./schema";
import { Warning } from "./warning";

type Authentication =
  MachineFormValues["protocols"]["ssh"]["authentication"]["method"];

export const AuthenticationSection = withFieldGroup({
  defaultValues: initialValues().protocols.ssh.authentication.method,
  props: { saved: undefined as SshAuthMethodResponse | undefined },
  render: function AuthenticationSectionFields({ group, saved }) {
    const titleId = useId();
    const title = (
      <div id={titleId} className="text-sm font-medium">
        {m.machine_field_authentication()}
      </div>
    );

    return (
      <group.Subscribe selector={(state) => state.values.replace}>
        {(replacing) =>
          saved && !replacing ? (
            <FieldSet aria-labelledby={titleId} className="pt-2">
              {title}
              <CurrentAuthentication
                authentication={saved}
                onReplace={() => group.setFieldValue("replace", true)}
              />
            </FieldSet>
          ) : (
            <group.Field name="kind">
              {(field) => (
                <Tabs
                  value={field.state.value}
                  onValueChange={(value) =>
                    field.handleChange(value as Authentication["kind"])
                  }
                >
                  <FieldSet aria-labelledby={titleId} className="pt-2">
                    <div className="flex flex-wrap items-center gap-3">
                      {title}
                      <TabsList aria-labelledby={titleId}>
                        <TabsTrigger value="keypair">
                          {m.machine_form_authentication_keypair()}
                        </TabsTrigger>
                        <TabsTrigger value="password">
                          {m.machine_authentication_password()}
                        </TabsTrigger>
                      </TabsList>
                    </div>
                    {saved && (
                      <Warning
                        title={
                          saved.kind === "password"
                            ? m.machine_form_replace_password_title()
                            : m.machine_form_replace_key_title()
                        }
                        description={m.machine_form_replace_description()}
                        action={
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => {
                              for (const name of [
                                "kind",
                                "source",
                                "password",
                                "replace",
                              ] as const) {
                                group.resetField(name);
                              }
                            }}
                          >
                            {m.machine_form_replace_keep()}
                          </Button>
                        }
                      />
                    )}
                    <TabsContent
                      value="keypair"
                      className="flex flex-col gap-4"
                    >
                      <group.Field name="source.kind">
                        {(field) => (
                          <RadioGroup
                            value={field.state.value}
                            onValueChange={(value) =>
                              field.handleChange(
                                value as Authentication["source"]["kind"],
                              )
                            }
                            className="grid-cols-1 md:grid-cols-2"
                          >
                            <ChoiceCard
                              value="managed"
                              title={m.machine_form_key_managed_title()}
                              description={m.machine_form_key_managed_description()}
                            />
                            <ChoiceCard
                              value="imported"
                              title={m.machine_form_key_imported_title()}
                              description={m.machine_form_key_imported_description()}
                            />
                          </RadioGroup>
                        )}
                      </group.Field>

                      <group.Subscribe
                        selector={(state) => state.values.source.kind}
                      >
                        {(source) =>
                          source === "imported" && (
                            <group.AppField name="source.private_key">
                              {(field) => (
                                <field.TextareaField
                                  label={m.machine_form_private_key_label()}
                                  description={m.machine_form_private_key_description()}
                                  className="font-mono text-xs"
                                  placeholder={
                                    // eslint-disable-next-line i18next/no-literal-string -- an example key, not language
                                    "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAAB...\n-----END OPENSSH PRIVATE KEY-----"
                                  }
                                />
                              )}
                            </group.AppField>
                          )
                        }
                      </group.Subscribe>
                    </TabsContent>
                    <TabsContent value="password">
                      <group.AppField name="password">
                        {(field) => (
                          <field.TextField
                            label={m.machine_form_password_label()}
                            type="password"
                            autoComplete="new-password"
                          />
                        )}
                      </group.AppField>
                    </TabsContent>
                  </FieldSet>
                </Tabs>
              )}
            </group.Field>
          )
        }
      </group.Subscribe>
    );
  },
});

function CurrentAuthentication({
  authentication,
  onReplace,
}: {
  authentication: SshAuthMethodResponse;
  onReplace: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-lg border p-3">
      <div className="flex min-w-0 flex-[1_1_12rem] items-center gap-3">
        {authentication.kind === "password" ? (
          <Lock className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <KeyRound className="size-4 shrink-0 text-muted-foreground" />
        )}
        <div className="flex flex-col gap-0.5">
          <span className="text-sm font-medium">
            {authentication.kind === "password"
              ? m.machine_authentication_password()
              : authentication.source === "managed"
                ? m.machine_form_current_key_managed()
                : m.machine_form_current_key_imported()}
          </span>
          <span className="text-sm text-muted-foreground">
            {m.machine_form_current_description()}
          </span>
        </div>
      </div>
      <Button
        variant="outline"
        size="sm"
        className="ml-auto"
        onClick={onReplace}
      >
        {m.machine_form_current_replace()}
      </Button>
    </div>
  );
}
