import { useMutation } from "@tanstack/react-query";
import {
  createFileRoute,
  useNavigate,
  useRouter,
} from "@tanstack/react-router";
import { Fingerprint, LogOut, Unlink } from "lucide-react";

import { ErrorAlert } from "@/components/error-alert";
import {
  toFormValidationError,
  useAppForm,
  validationLogic,
} from "@/components/form";
import { Alert, AlertDescription } from "@/components/shadcn/alert";
import { Button } from "@/components/shadcn/button";
import { Field, FieldGroup, FieldSeparator } from "@/components/shadcn/field";
import {
  CredentialErrorAlert,
  passkeyErrorMessage,
} from "@/components/sign-in";
import { NewPasswordFields } from "@/features/account/components/new-password-fields";
import { recoverAccountMutation } from "@/generated/api/@tanstack/react-query.gen";
import { recoverAccount, recoveryOptions } from "@/generated/api/sdk.gen";
import type { RecoveryOptionsResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";
import { useSignOut } from "@/lib/auth";
import { createPasskey, passkeysSupported } from "@/lib/passkeys";
import {
  AuthMessage,
  PageTitle,
  ServerUnreachable,
} from "@/routes/_auth/-auth-page";
import { recoveryToken } from "@/routes/_auth/-recovery-link";

export const Route = createFileRoute("/_auth/recover-account")({
  loader: ({ location }) => {
    const token = recoveryToken(location.hash);
    if (!token) return undefined;

    return recoveryOptions({ auth: token, throwOnError: true }).then(
      ({ data }) => ({ token, methods: data.methods }),
      (error: unknown) => {
        if (isApiError(error) && error.status === 400) return undefined;
        throw error;
      },
    );
  },

  errorComponent: ServerUnreachable,
  component: RouteComponent,
});

function RouteComponent() {
  const recovery = Route.useLoaderData();
  const signedInUser = Route.useRouteContext({
    select: (context) => context.signedInUser,
  });

  return (
    <>
      <title>{m.page_title({ page: m.recover_account_page_title() })}</title>
      {recovery ? (
        <div className="flex flex-col gap-6 p-6 md:p-8">
          <PageTitle
            title={m.recover_account_title()}
            description={m.recover_account_description()}
          />
          {signedInUser ? (
            <SignOutStep username={signedInUser.username} />
          ) : (
            <RecoveryForm token={recovery.token} methods={recovery.methods} />
          )}
        </div>
      ) : (
        <AuthMessage
          Icon={Unlink}
          tone="destructive"
          title={m.recover_account_link_does_not_work()}
          description={m.recover_account_link_does_not_work_description()}
        />
      )}
    </>
  );
}

function SignOutStep({ username }: { username: string }) {
  const router = useRouter();
  const signOut = useSignOut();

  return (
    <div className="flex flex-col gap-4">
      {signOut.error && (
        <ErrorAlert
          title={m.recover_account_sign_out_error()}
          error={signOut.error}
        />
      )}
      <Alert>
        <LogOut />
        <AlertDescription>
          {m.recover_account_signed_in({ username })}
        </AlertDescription>
      </Alert>
      <Button
        disabled={signOut.isPending}
        onClick={() =>
          signOut.mutate({}, { onSuccess: () => void router.invalidate() })
        }
      >
        {m.recover_account_sign_out()}
      </Button>
    </div>
  );
}

function RecoveryForm({
  token,
  methods,
}: {
  token: string;
  methods: RecoveryOptionsResponse["methods"];
}) {
  const navigate = useNavigate();
  const goToSignIn = (recovered: "password" | "passkey") =>
    navigate({ to: "/login", search: { recovered }, replace: true });
  const setPassword = useMutation(recoverAccountMutation());

  const addPasskey = useMutation({
    mutationFn: async () => {
      const { data: started } = await recoverAccount({
        auth: token,
        body: { action: "start_passkey_registration" },
        throwOnError: true,
      });
      if (started.kind !== "passkey_creation_options")
        throw new Error(`Unexpected recovery answer: ${started.kind}`);
      await recoverAccount({
        auth: token,
        body: {
          action: "finish_passkey_registration",
          response: await createPasskey(started.options),
        },
        throwOnError: true,
      });
    },
    onSuccess: () => goToSignIn("passkey"),
    retry: false,
    throwOnError: (error) =>
      !isApiError(error) && passkeyErrorMessage(error) === undefined,
  });

  const form = useAppForm({
    defaultValues: { password: "", confirm_password: "" },
    validationLogic,
    validators: {
      onSubmitAsync: async ({ value, formApi }) => {
        try {
          await setPassword.mutateAsync({
            auth: token,
            body: { action: "set_password", password: value.password },
          });
        } catch (error) {
          if (!isApiError(error)) throw error;
          return toFormValidationError(formApi, error);
        }
      },
    },
    onSubmit: () => goToSignIn("password"),
  });

  return (
    <form.AppForm>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void form.handleSubmit();
        }}
      >
        <FieldGroup>
          <form.SubmitError title={m.recover_account_error()} />
          <CredentialErrorAlert
            title={m.recover_account_passkey_error()}
            error={addPasskey.error}
          />

          <NewPasswordFields
            form={form}
            fields={{
              password: "password",
              confirm_password: "confirm_password",
            }}
          />

          <Field>
            <form.SubmitButton>{m.recover_account_submit()}</form.SubmitButton>
          </Field>

          {methods.includes("passkey") && passkeysSupported() && (
            <>
              <FieldSeparator className="*:data-[slot=field-separator-content]:bg-card">
                {m.login_or()}
              </FieldSeparator>
              <Button
                type="button"
                variant="outline"
                disabled={addPasskey.isPending}
                onClick={() => addPasskey.mutate()}
              >
                <Fingerprint />
                {m.recover_account_use_passkey()}
              </Button>
            </>
          )}
        </FieldGroup>
      </form>
    </form.AppForm>
  );
}
