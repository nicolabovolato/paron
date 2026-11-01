import type { ToastManagerAddOptions } from "@base-ui/react/toast";

import { CopyButton } from "@/components/code-block";
import { m } from "@/generated/paraglide/messages";
import {
  errorMessage,
  reportableRequestId,
  type ApiError,
} from "@/lib/api-error";

export function errorToast(title: string, error: ApiError) {
  const requestId = reportableRequestId(error);

  return {
    type: "error",
    title,
    description: (
      <>
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
      </>
    ),
  } satisfies ToastManagerAddOptions<object>;
}
