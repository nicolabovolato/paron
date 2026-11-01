import type { ApiError as ApiErrorBody } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export type ApiError = ApiErrorBody & {
  retryAfterMs?: number;
  requestId?: string;
} & ({ status: 0; aborted: boolean } | { status: number; aborted?: never });

export function toApiError(
  error: unknown,
  response: Response | undefined,
  request: Request | undefined,
) {
  if (!response) {
    const aborted = request?.signal.aborted ?? false;
    if (!aborted) console.error(error);

    return { status: 0, aborted } satisfies ApiError;
  }

  const retryAfterMs = millisecondsFrom(response.headers.get("retry-after"));
  const requestId = response.headers.get("x-request-id") ?? undefined;

  if (isApiError(error))
    return { ...error, retryAfterMs, requestId } satisfies ApiError;

  return {
    status: response.status,
    detail: (isPlainText(response) && String(error).trim()) || undefined,
    retryAfterMs,
    requestId,
  } satisfies ApiError;
}

export function isApiError(value: unknown): value is ApiError {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as ApiError).status === "number"
  );
}

export function errorMessage({ status, code, detail }: ApiError) {
  if (status === 429)
    return `${
      code === "too_many_failed_attempts"
        ? m.request_too_many_failed_attempts()
        : m.request_too_many()
    } ${m.request_retry_later()}`;
  if (detail) return detail.charAt(0).toUpperCase() + detail.slice(1);
  if (status === 0) return m.request_unreachable();

  return m.request_unexpected({ status });
}

export function reportableRequestId({ status, detail, requestId }: ApiError) {
  return status >= 500 && !detail ? requestId : undefined;
}

function isPlainText(response: Response) {
  return (
    response.headers.get("content-type")?.split(";")[0]?.trim() === "text/plain"
  );
}

function millisecondsFrom(retryAfter: string | null) {
  if (retryAfter === null) return undefined;

  if (/^\d+$/.test(retryAfter)) return Number(retryAfter) * 1000;

  const date = Date.parse(retryAfter);

  return Number.isNaN(date) ? undefined : Math.max(0, date - Date.now());
}
