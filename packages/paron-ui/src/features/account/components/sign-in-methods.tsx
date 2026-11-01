import { ParaglideMessage } from "@inlang/paraglide-js-react";
import type { LucideIcon } from "lucide-react";
import {
  Check,
  Fingerprint,
  KeyRound,
  ListChecks,
  Pencil,
  Smartphone,
  X,
} from "lucide-react";
import { useState, type ReactNode } from "react";
import { z } from "zod";

import type { Action } from "@/components/actions";
import { ActionButton } from "@/components/actions";
import { CardHeader } from "@/components/card";
import {
  toFormValidationError,
  useAppForm,
  validationLogic,
} from "@/components/form";
import { RelativeTime } from "@/components/relative-time";
import { Badge } from "@/components/shadcn/badge";
import { Button } from "@/components/shadcn/button";
import { Card, CardContent, CardTitle } from "@/components/shadcn/card";
import {
  Item,
  ItemActions,
  ItemContent,
  ItemDescription,
  ItemMedia,
  ItemTitle,
} from "@/components/shadcn/item";
import {
  useCredentialActions,
  usePasskeyActions,
} from "@/features/account/hooks/account-actions";
import { useUpdateCredentials } from "@/features/account/lib/api";
import type {
  CredentialsResponse,
  PasskeyCredentialResponse,
} from "@/generated/api/types.gen";
import { zPasskeyName } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

export function SignInMethods({
  credentials,
}: {
  credentials: CredentialsResponse;
}) {
  const actions = useCredentialActions(credentials);
  const password = credentials.password;
  const totp = credentials.totp;
  const recoveryCodes = credentials.recovery_codes;

  return (
    <Card>
      <CardHeader>
        <CardTitle>{m.account_sign_in_methods_title()}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <div className="flex flex-col gap-3">
          <Method
            Icon={KeyRound}
            title={m.password_title()}
            description={
              password ? (
                <ParaglideMessage
                  message={m.password_since}
                  inputs={{}}
                  markup={{
                    time: () => <RelativeTime iso={password.updated_at} />,
                  }}
                />
              ) : (
                m.password_off()
              )
            }
            action={actions.setPassword}
            removal={actions.removePassword}
          />

          <Method
            Icon={Smartphone}
            title={m.totp_title()}
            description={
              totp ? (
                <ParaglideMessage
                  message={m.totp_since}
                  inputs={{}}
                  markup={{
                    time: () => <RelativeTime iso={totp.created_at} />,
                    used: () => <RelativeTime iso={totp.last_used_at} />,
                  }}
                />
              ) : (
                m.totp_off()
              )
            }
            action={actions.setUpTotp}
            removal={actions.removeTotp}
          />

          <Method
            Icon={Fingerprint}
            title={m.passkeys_title()}
            action={actions.addPasskey}
          >
            {credentials.passkeys.length > 0 && (
              <ul className="border-t">
                {credentials.passkeys.map((passkey) => (
                  <PasskeyRow key={passkey.id} passkey={passkey} />
                ))}
              </ul>
            )}
          </Method>

          {recoveryCodes && (
            <Method
              Icon={ListChecks}
              title={m.recovery_codes_item_title()}
              description={m.recovery_codes_left({
                count: recoveryCodes.remaining,
              })}
              action={actions.generateRecoveryCodes}
            />
          )}
        </div>
      </CardContent>
    </Card>
  );
}

function Method({
  Icon,
  title,
  description,
  action,
  removal,
  children,
}: {
  Icon: LucideIcon;
  title: string;
  description?: ReactNode;
  action?: Action;
  removal?: Action;
  children?: ReactNode;
}) {
  return (
    <div className="overflow-hidden rounded-lg border">
      <Item>
        <ItemMedia variant="icon">
          <Icon />
        </ItemMedia>
        <ItemContent>
          <ItemTitle>{title}</ItemTitle>
          {description && <ItemDescription>{description}</ItemDescription>}
        </ItemContent>
        <ItemActions>
          {action && <ActionButton action={action} />}
          {removal && (
            <ActionButton action={removal} variant="ghost" iconOnly />
          )}
        </ItemActions>
      </Item>
      {children}
    </div>
  );
}

function PasskeyRow({ passkey }: { passkey: PasskeyCredentialResponse }) {
  const actions = usePasskeyActions(passkey);
  const [renaming, setRenaming] = useState(false);

  return (
    <li className="flex min-h-14 items-center gap-2 bg-muted/40 py-2 pr-3 pl-9.5 not-last:border-b">
      {renaming ? (
        <PasskeyNameForm passkey={passkey} onDone={() => setRenaming(false)} />
      ) : (
        <>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <p className="flex items-center gap-2 text-sm font-medium">
              <span className="truncate">{passkey.name}</span>
              {passkey.backed_up && (
                <Badge variant="secondary">{m.passkey_synced()}</Badge>
              )}
            </p>
            <p className="text-sm text-muted-foreground">
              {passkey.last_used_at ? (
                <>
                  {m.passkey_last_used()}{" "}
                  <RelativeTime iso={passkey.last_used_at} />
                </>
              ) : (
                m.passkey_never_used()
              )}
            </p>
          </div>
          <ActionButton
            action={{
              label: m.passkey_action_rename({ name: passkey.name }),
              Icon: Pencil,
              kind: "perform",
              perform: () => {
                setRenaming(true);
                return true;
              },
            }}
            variant="ghost"
            iconOnly
          />
          <ActionButton action={actions.remove} variant="ghost" iconOnly />
        </>
      )}
    </li>
  );
}

const nameSchema = z.object({ name: z.string().trim().pipe(zPasskeyName) });

function PasskeyNameForm({
  passkey,
  onDone,
}: {
  passkey: PasskeyCredentialResponse;
  onDone: () => void;
}) {
  const updateCredentials = useUpdateCredentials();

  const form = useAppForm({
    defaultValues: { name: passkey.name },
    validationLogic,
    validators: {
      onDynamic: nameSchema,
      onSubmitAsync: async ({ value, formApi }) => {
        try {
          await updateCredentials.mutateAsync({
            body: {
              action: "rename_passkey",
              id: passkey.id,
              ...nameSchema.parse(value),
            },
          });
          onDone();
        } catch (error) {
          if (!isApiError(error)) throw error;
          return toFormValidationError(formApi, error);
        }
      },
    },
  });

  return (
    <form.AppForm>
      <form
        className="flex min-w-0 flex-1 flex-col gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") onDone();
        }}
      >
        <div className="flex items-start gap-2">
          <form.AppField name="name">
            {(field) => (
              <field.TextField
                label={m.passkey_name_label()}
                labelHidden
                autoComplete="off"
                autoFocus
              />
            )}
          </form.AppField>
          <form.SubmitButton
            size="icon-sm"
            aria-label={m.passkey_action_rename({ name: passkey.name })}
            className="mt-0.5"
          >
            <Check />
          </form.SubmitButton>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={m.passkey_rename_cancel()}
            className="mt-0.5"
            onClick={onDone}
          >
            <X />
          </Button>
        </div>
        <form.SubmitError title={m.passkey_rename_error()} />
      </form>
    </form.AppForm>
  );
}
