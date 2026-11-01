import type { AnyFormApi } from "@tanstack/react-form";

import type { ApiError } from "@/lib/api-error";

/**
 * A failed request as a submit validator's result: each validation message on
 * its field, and the error itself on the form when some part of it has no
 * field to show it.
 */
export function toFormValidationError(
  form: Pick<AnyFormApi, "getFieldMeta">,
  error: ApiError,
) {
  const messages = Object.entries(error.validation_errors ?? {});
  const placed = messages.filter(
    ([path]) => form.getFieldMeta(path) !== undefined,
  );

  return {
    form:
      placed.length > 0 && placed.length === messages.length
        ? undefined
        : error,
    fields: Object.fromEntries(
      placed.map(([path, lines]) => [path, { message: lines.join(" ") }]),
    ),
  };
}
