import { AlertCircle } from "lucide-react";

import { CopyButton } from "@/components/code-block";
import { Alert, AlertDescription, AlertTitle } from "@/components/shadcn/alert";
import { m } from "@/generated/paraglide/messages";
import {
  errorMessage,
  reportableRequestId,
  type ApiError,
} from "@/lib/api-error";

export function ErrorAlert({
  title,
  error,
}: {
  title: string;
  error: ApiError;
}) {
  const requestId = reportableRequestId(error);

  return (
    <Alert variant="destructive">
      <AlertCircle />
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>
        {errorMessage(error)}
        {requestId && (
          <span className="mt-2 block">
            <CopyButton
              value={requestId}
              label={m.request_id_copy()}
              variant="outline"
              size="xs"
            />
          </span>
        )}
      </AlertDescription>
    </Alert>
  );
}
