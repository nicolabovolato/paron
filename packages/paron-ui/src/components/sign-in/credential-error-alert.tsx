import { Fingerprint } from "lucide-react";

import { ErrorAlert } from "@/components/error-alert";
import { Alert, AlertDescription } from "@/components/shadcn/alert";
import { isApiError } from "@/lib/api-error";

import { passkeyErrorMessage } from "./passkey-error";

export function PasskeyAlert({ message }: { message: string }) {
  return (
    <Alert>
      <Fingerprint />
      <AlertDescription>{message}</AlertDescription>
    </Alert>
  );
}

/** A refused request or a browser passkey error. `title` is for the request. */
export function CredentialErrorAlert({
  title,
  error,
}: {
  title: string;
  error: unknown;
}) {
  if (isApiError(error)) return <ErrorAlert title={title} error={error} />;
  const message = passkeyErrorMessage(error);
  return message && <PasskeyAlert message={message} />;
}
