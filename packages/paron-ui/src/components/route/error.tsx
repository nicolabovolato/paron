import { useRouter, type ErrorComponentProps } from "@tanstack/react-router";
import { PathParamError } from "@tanstack/router-core";
import { AlertCircle } from "lucide-react";

import { CopyButton } from "@/components/code-block";
import { RouteForbidden } from "@/components/route/forbidden";
import { RouteNotFound } from "@/components/route/not-found";
import { Button } from "@/components/shadcn/button";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/shadcn/empty";
import { m } from "@/generated/paraglide/messages";
import { ForbiddenError } from "@/hooks/permissions";
import { errorMessage, isApiError, reportableRequestId } from "@/lib/api-error";

export function RouteError({ error, reset }: ErrorComponentProps) {
  const router = useRouter();
  const apiError = isApiError(error) ? error : undefined;
  const requestId = apiError && reportableRequestId(apiError);

  if (error instanceof PathParamError || apiError?.status === 404)
    return <RouteNotFound />;
  if (error instanceof ForbiddenError || apiError?.status === 403)
    return <RouteForbidden />;

  return (
    <Empty className="border">
      <EmptyHeader>
        <EmptyMedia
          variant="icon"
          className="bg-destructive/10 text-destructive"
        >
          <AlertCircle />
        </EmptyMedia>
        <EmptyTitle>
          <h1>{m.route_error_title()}</h1>
        </EmptyTitle>
        {apiError && (
          <EmptyDescription>{errorMessage(apiError)}</EmptyDescription>
        )}
      </EmptyHeader>
      <EmptyContent>
        <Button
          variant="outline"
          onClick={() => {
            reset();
            void router.invalidate();
          }}
        >
          {m.route_error_retry()}
        </Button>
        {requestId && (
          <CopyButton
            value={requestId}
            label={m.request_id_copy()}
            variant="outline"
            size="xs"
          />
        )}
        {!apiError && error instanceof Error && (
          <CopyButton
            value={error.stack ?? `${error.name}: ${error.message}`}
            label={m.route_error_copy_details()}
            variant="outline"
            size="xs"
          />
        )}
      </EmptyContent>
    </Empty>
  );
}
