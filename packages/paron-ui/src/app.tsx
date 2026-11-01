import { ConnectionBanner } from "@/components/connection-banner";
import { RouteError } from "@/components/route/error";
import { RouteNotFound } from "@/components/route/not-found";
import { RoutePending } from "@/components/route/pending";
import { TooltipProvider } from "@/components/shadcn/tooltip";
import { ThemeProvider } from "@/components/theme-provider";
import { readSession } from "@/generated/api/sdk.gen";
import type { Permission } from "@/generated/api/types.gen";
import { routeTree } from "@/generated/routeTree.gen";
import { ClockProvider } from "@/hooks/now";
import type { ApiError } from "@/lib/api-error";
import { authQueries } from "@/lib/auth";
import {
  MutationCache,
  QueryCache,
  QueryClient,
  QueryClientProvider,
} from "@tanstack/react-query";
import {
  createRouter,
  RouterProvider,
  type RouterHistory,
} from "@tanstack/react-router";

export function App({ queryClient, router }: ReturnType<typeof createApp>) {
  return (
    <ThemeProvider>
      <QueryClientProvider client={queryClient}>
        <ClockProvider>
          <TooltipProvider>
            <ConnectionBanner />
            <RouterProvider router={router} />
          </TooltipProvider>
        </ClockProvider>
      </QueryClientProvider>
    </ThemeProvider>
  );
}

const MIN_RETRY_DELAY_MS = 1_000;
const MAX_RETRY_DELAY_MS = 3_000;

// eslint-disable-next-line react-refresh/only-export-components
export function createApp(history?: RouterHistory) {
  const queryClient = new QueryClient({
    queryCache: new QueryCache({ onError: endSessionOnUnauthorized }),
    mutationCache: new MutationCache({
      onSuccess: (_data, _variables, _result, mutation, { client }) =>
        mutation.meta?.invalidates === false
          ? undefined
          : client.invalidateQueries(),
      onError: endSessionOnUnauthorized,
    }),
    defaultOptions: {
      queries: {
        retry: (failureCount, error) =>
          failureCount < 2 &&
          (error.retryAfterMs ?? 0) <= MAX_RETRY_DELAY_MS &&
          (error.status === 0 || error.status === 429 || error.status >= 500),
        retryDelay: (failureCount, error) =>
          Math.max(
            error.retryAfterMs ?? 1000 * 2 ** failureCount,
            MIN_RETRY_DELAY_MS,
          ),
      },
      mutations: {
        retry: (failureCount, error) =>
          failureCount < 2 &&
          (error.retryAfterMs ?? 0) <= MAX_RETRY_DELAY_MS &&
          error.status === 429,
        retryDelay: (failureCount, error) =>
          Math.max(
            error.retryAfterMs ?? 1000 * 2 ** failureCount,
            MIN_RETRY_DELAY_MS,
          ),
      },
    },
  });

  const router = createRouter({
    routeTree,
    context: { queryClient },
    history,
    scrollRestoration: true,
    defaultPreload: "intent",
    defaultPreloadStaleTime: 0,
    defaultPendingMinMs: 500,
    defaultPendingMs: 1000,
    defaultErrorComponent: RouteError,
    defaultPendingComponent: RoutePending,
    defaultNotFoundComponent: RouteNotFound,
    defaultRemountDeps: ({ params }: { params: unknown }) => params,
  });

  let sessionCheck: Promise<void> | undefined;
  function endSessionOnUnauthorized(error: ApiError) {
    if (
      error.status !== 401 ||
      sessionCheck ||
      !queryClient.getQueryData(authQueries.me().queryKey) ||
      router.latestLocation.pathname ===
        router.routesById["/_auth/login"].fullPath
    )
      return;

    sessionCheck = readSession()
      .then(({ response }) => {
        if (response?.status !== 401) return;

        void router.navigate({
          to: "/login",
          search: { redirect: router.state.location.href, expired: true },
          replace: true,
          ignoreBlocker: true,
        });
      })
      .finally(() => {
        sessionCheck = undefined;
      });
  }

  return { queryClient, router };
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof createApp>["router"];
  }

  interface StaticDataRouteOption {
    permission?: Permission;
    width?: "narrow" | "wide" | "full";
    height?: "auto" | "screen";
    notFound?: { title: () => string; description: () => string };
  }
}

declare module "@tanstack/react-query" {
  interface Register {
    defaultError: ApiError;
    mutationMeta: { invalidates?: false };
  }
}
