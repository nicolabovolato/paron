import { useSelector } from "@tanstack/react-form";

import { ErrorAlert } from "@/components/error-alert";
import { isApiError } from "@/lib/api-error";

import { useFormContext } from "./context";

export function SubmitError({ title }: { title: string }) {
  const form = useFormContext();
  const error = useSelector(form.store, (state) => {
    const submitError: unknown = state.errorMap.onSubmit;
    return isApiError(submitError) ? submitError : undefined;
  });

  return error && <ErrorAlert title={title} error={error} />;
}
