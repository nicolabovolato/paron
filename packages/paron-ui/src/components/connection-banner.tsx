import { queryOptions, useQuery, useQueryClient } from "@tanstack/react-query";
import { RotateCw, Unplug } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/shadcn/button";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";

const MIN_CHECK_DELAY_MS = 1_000;
const MAX_CHECK_DELAY_MS = 30_000;

export function ConnectionBanner() {
  const queryClient = useQueryClient();
  const [connectionLost, setConnectionLost] = useState(false);

  useEffect(() => {
    function failed(error: unknown) {
      if (!isApiError(error) || (error.status === 0 && error.aborted)) return;

      setConnectionLost(error.status === 0);
    }

    const unsubscribeQueries = queryClient
      .getQueryCache()
      .subscribe((event) => {
        if (event.type !== "updated") return;
        if (event.action.type === "success" && !event.action.manual)
          setConnectionLost(false);
        if (event.action.type === "error") failed(event.action.error);
      });
    const unsubscribeMutations = queryClient
      .getMutationCache()
      .subscribe((event) => {
        if (event.type !== "updated") return;
        if (event.action.type === "success") setConnectionLost(false);
        if (event.action.type === "error") failed(event.action.error);
      });

    return () => {
      unsubscribeQueries();
      unsubscribeMutations();
    };
  }, [queryClient]);

  useQuery(
    queryOptions({
      queryKey: ["health"],
      queryFn: ({ signal }) =>
        fetch(new URL("/health", location.origin), {
          cache: "no-store",
          signal,
        }).then(() => null),
      enabled: connectionLost,
      retry: true,
      retryDelay: (failureCount) =>
        Math.min(MIN_CHECK_DELAY_MS * 2 ** failureCount, MAX_CHECK_DELAY_MS),
    }),
  );

  return (
    <div
      data-slot="connection-banner"
      role="alert"
      hidden={!connectionLost}
      className="sticky top-0 z-40 flex h-(--connection-banner-height) items-center justify-center gap-4 overflow-hidden bg-destructive px-4 text-xs whitespace-nowrap text-background transition-[display] transition-discrete duration-(--connection-banner-duration) dark:bg-[color-mix(in_oklch,var(--destructive)_35%,var(--background))] dark:text-foreground"
    >
      <p className="flex items-center gap-1.5 font-semibold">
        <Unplug aria-hidden className="size-3.5 shrink-0" />
        {m.connection_lost_title()}
      </p>
      <p className="hidden sm:block">{m.connection_lost_description()}</p>
      <Button
        variant="link"
        className="h-auto gap-1.5 rounded-sm p-0 text-xs text-current underline hover:decoration-2 focus-visible:ring-current/40"
        onClick={() => location.reload()}
      >
        <RotateCw className="size-3" />
        {m.connection_lost_reload()}
      </Button>
    </div>
  );
}
