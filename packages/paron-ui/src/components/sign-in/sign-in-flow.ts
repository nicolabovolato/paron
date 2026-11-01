import { useMutation } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";

import { startSignIn, verifySignIn } from "@/generated/api/sdk.gen";
import type {
  AcceptedCredentialResponse,
  PasskeyRequestOptionsJson,
  SignInCredentialRequest,
} from "@/generated/api/types.gen";
import { isApiError } from "@/lib/api-error";
import { getPasskey, passkeysSupported } from "@/lib/passkeys";

import { passkeyErrorMessage } from "./passkey-error";

export type SignInStep = {
  number: number;
  accepted: AcceptedCredentialResponse[];
};

export function useSignInFlow({
  stepUp = false,
  onSignedIn,
}: {
  stepUp?: boolean;
  onSignedIn: () => unknown;
}) {
  const [step, setStep] = useState<SignInStep>();
  // Each new value starts a new sign-in on the server.
  const [round, setRound] = useState(0);
  const browserRequest = useRef<AbortController>(undefined);

  const { mutate: start, error: startError } = useMutation({
    mutationFn: async (signal: AbortSignal) => {
      const { data } = await startSignIn({
        query: stepUp ? { step_up: true } : undefined,
        signal,
        throwOnError: true,
      });
      return data.accepted;
    },
    onSuccess: (accepted) => setStep({ number: 1, accepted }),
    meta: { invalidates: false },
  });

  const verify = useMutation({
    mutationFn: async (credential: () => Promise<SignInCredentialRequest>) => {
      const { data } = await verifySignIn({
        body: await credential(),
        throwOnError: true,
      });
      return data;
    },
    onSuccess: async ({ accepted }) => {
      if (accepted)
        setStep((current) => ({
          number: (current?.number ?? 0) + 1,
          accepted,
        }));
      else await onSignedIn();
    },
    onError: (error) => {
      if (isApiError(error) && error.status === 401) startOver();
    },
    retry: false,
    throwOnError: (error) =>
      !isApiError(error) && passkeyErrorMessage(error) === undefined,
    meta: stepUp ? undefined : { invalidates: false },
  });

  useEffect(() => {
    const controller = new AbortController();
    start(controller.signal);
    return () => controller.abort();
  }, [start, round]);

  function startOver() {
    setStep(undefined);
    setRound((current) => current + 1);
  }

  function submit(
    credential: () =>
      SignInCredentialRequest | Promise<SignInCredentialRequest>,
  ) {
    browserRequest.current?.abort();
    return new Promise<void>((resolve) => {
      verify.mutate(async () => credential(), { onSettled: () => resolve() });
    });
  }

  async function offerPasskeys(
    options: PasskeyRequestOptionsJson,
    signal: AbortSignal,
  ) {
    if (
      !passkeysSupported() ||
      !(await PublicKeyCredential.isConditionalMediationAvailable())
    )
      return;
    const response = await getPasskey(options, {
      mediation: "conditional",
      signal,
    });
    await submit(() => ({ kind: "passkey", response }));
  }

  function autofill(options: PasskeyRequestOptionsJson) {
    const controller = new AbortController();
    browserRequest.current = controller;
    offerPasskeys(options, controller.signal).catch((error: unknown) => {
      if (!(error instanceof DOMException)) throw error;
    });
    return () => controller.abort();
  }

  return {
    step,
    startError,
    error: verify.error,
    pending: verify.isPending,
    submit,
    autofill,
    restart: () => {
      verify.reset();
      startOver();
    },
  };
}

export type SignInFlow = ReturnType<typeof useSignInFlow>;
