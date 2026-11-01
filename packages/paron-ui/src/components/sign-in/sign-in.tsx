import { REGEXP_ONLY_DIGITS, REGEXP_ONLY_DIGITS_AND_CHARS } from "input-otp";
import { ArrowLeft, Fingerprint } from "lucide-react";
import { type ReactNode, useEffect, useEffectEvent, useState } from "react";
import { z } from "zod";

import { ErrorAlert } from "@/components/error-alert";
import { useAppForm, validationLogic } from "@/components/form";
import { Button } from "@/components/shadcn/button";
import { Field, FieldGroup, FieldSeparator } from "@/components/shadcn/field";
import { Spinner } from "@/components/shadcn/spinner";
import type { PasskeyRequestOptionsJson } from "@/generated/api/types.gen";
import { zPassword, zUsername } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { getPasskey, passkeysSupported } from "@/lib/passkeys";

import { CredentialErrorAlert, PasskeyAlert } from "./credential-error-alert";
import {
  type SignInFlow,
  type SignInStep,
  useSignInFlow,
} from "./sign-in-flow";

const passwordSchema = z.object({
  username: z.string().trim().pipe(zUsername),
  password: zPassword,
});

const codeSchema = z.object({ code: z.string().trim().toLowerCase().min(1) });

type CodeKind = "totp" | "recovery_code";

export function SignIn({
  username,
  onSignedIn,
  heading,
  nextStepHeading,
  notice,
}: {
  username?: string;
  onSignedIn: () => unknown;
  heading: ReactNode;
  nextStepHeading: ReactNode;
  notice?: ReactNode;
}) {
  const flow = useSignInFlow({ stepUp: username !== undefined, onSignedIn });

  if (flow.startError)
    return (
      <FieldGroup>
        {heading}
        <ErrorAlert title={m.login_error()} error={flow.startError} />
        <Button type="button" variant="outline" onClick={flow.restart}>
          {m.sign_in_unavailable_retry()}
        </Button>
      </FieldGroup>
    );

  if (!flow.step)
    return (
      <FieldGroup>
        {heading}
        <Spinner className="mx-auto" />
      </FieldGroup>
    );

  const firstStep = flow.step.number === 1;

  return (
    <FieldGroup key={flow.step.number}>
      {firstStep ? heading : nextStepHeading}
      <CredentialErrorAlert title={m.login_error()} error={flow.error} />
      {firstStep && !flow.error && notice}
      <Step step={flow.step} flow={flow} username={username} />
      {!firstStep && (
        <Button type="button" variant="ghost" onClick={flow.restart}>
          <ArrowLeft />
          {m.second_factor_back()}
        </Button>
      )}
    </FieldGroup>
  );
}

function Step({
  step,
  flow,
  username,
}: {
  step: SignInStep;
  flow: SignInFlow;
  username: string | undefined;
}) {
  const kinds = step.accepted.map((credential) => credential.kind);
  const codeKinds = kinds.filter(
    (kind): kind is CodeKind => kind === "totp" || kind === "recovery_code",
  );
  const passkeyOptions = step.accepted.find(
    (credential) => credential.kind === "passkey",
  )?.options;
  const asksForPassword = kinds.includes("password");
  const asksForSecret = asksForPassword || codeKinds.length > 0;
  const autofillOptions =
    username === undefined && asksForPassword ? passkeyOptions : undefined;

  const offerAutofill = useEffectEvent(flow.autofill);
  useEffect(
    () => autofillOptions && offerAutofill(autofillOptions),
    [autofillOptions],
  );

  return (
    <>
      {asksForPassword && <PasswordForm flow={flow} username={username} />}
      {codeKinds.length > 0 && <CodeForm flow={flow} kinds={codeKinds} />}
      {passkeyOptions && passkeysSupported() && (
        <>
          {asksForSecret && (
            <FieldSeparator className="*:data-[slot=field-separator-content]:bg-card">
              {m.login_or()}
            </FieldSeparator>
          )}
          <PasskeyButton
            flow={flow}
            options={passkeyOptions}
            primary={!asksForSecret}
          />
        </>
      )}
      {passkeyOptions && !passkeysSupported() && !asksForSecret && (
        <PasskeyAlert message={m.passkey_unavailable()} />
      )}
    </>
  );
}

function PasswordForm({
  flow,
  username,
}: {
  flow: SignInFlow;
  username: string | undefined;
}) {
  const form = useAppForm({
    defaultValues: { username: username ?? "", password: "" },
    validationLogic,
    validators: { onDynamic: passwordSchema },
    onSubmit: ({ value }) =>
      flow.submit(() => ({ kind: "password", ...passwordSchema.parse(value) })),
  });

  return (
    <form.AppForm>
      <form
        className="flex flex-col gap-7"
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
      >
        {username === undefined ? (
          <form.AppField name="username">
            {(field) => (
              <field.TextField
                label={m.login_username_label()}
                autoComplete="username webauthn"
                autoFocus
              />
            )}
          </form.AppField>
        ) : (
          <input
            className="hidden"
            name="username"
            autoComplete="username"
            value={username}
            readOnly
          />
        )}
        <form.AppField name="password">
          {(field) => (
            <field.TextField
              label={m.login_password_label()}
              type="password"
              autoComplete="current-password"
              autoFocus={username !== undefined}
            />
          )}
        </form.AppField>
        <Field>
          <form.SubmitButton>{m.login_submit()}</form.SubmitButton>
        </Field>
      </form>
    </form.AppForm>
  );
}

function CodeForm({ flow, kinds }: { flow: SignInFlow; kinds: CodeKind[] }) {
  const [kind, setKind] = useState<CodeKind>(
    kinds.includes("totp") ? "totp" : "recovery_code",
  );
  const other = kinds.find((candidate) => candidate !== kind);

  const form = useAppForm({
    defaultValues: { code: "" },
    validationLogic,
    validators: { onDynamic: codeSchema },
    onSubmit: ({ value }) =>
      flow.submit(() => ({ kind, code: codeSchema.parse(value).code })),
  });

  return (
    <form.AppForm>
      <form
        className="flex flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
      >
        <form.AppField name="code">
          {(field) =>
            kind === "totp" ? (
              <field.CodeField
                key="totp"
                centered
                label={m.second_factor_code_label()}
                groups={[3, 3]}
                pattern={REGEXP_ONLY_DIGITS}
                inputMode="numeric"
                autoComplete="one-time-code"
                autoFocus
              />
            ) : (
              <field.CodeField
                key="recovery_code"
                centered
                label={m.second_factor_recovery_code_label()}
                groups={[5, 5]}
                pattern={REGEXP_ONLY_DIGITS_AND_CHARS}
                inputMode="text"
                autoComplete="off"
                autoFocus
              />
            )
          }
        </form.AppField>
        <Field>
          <form.SubmitButton>{m.second_factor_submit()}</form.SubmitButton>
        </Field>
        {other && (
          <Button
            type="button"
            variant="link"
            onClick={() => {
              setKind(other);
              form.reset();
            }}
          >
            {other === "totp"
              ? m.second_factor_use_app()
              : m.second_factor_use_recovery_code()}
          </Button>
        )}
      </form>
    </form.AppForm>
  );
}

function PasskeyButton({
  flow,
  options,
  primary,
}: {
  flow: SignInFlow;
  options: PasskeyRequestOptionsJson;
  primary: boolean;
}) {
  return (
    <Button
      type="button"
      variant={primary ? "default" : "outline"}
      disabled={flow.pending}
      onClick={() =>
        void flow.submit(async () => ({
          kind: "passkey",
          response: await getPasskey(options),
        }))
      }
    >
      <Fingerprint />
      {m.login_with_passkey()}
    </Button>
  );
}
