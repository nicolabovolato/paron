import { createFileRoute, Outlet } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";

import bannerUrl from "@/assets/banner.svg";
import paronUrl from "@/assets/paron.svg";
import { Card, CardContent } from "@/components/shadcn/card";
import { m } from "@/generated/paraglide/messages";
import { isApiError } from "@/lib/api-error";
import { authQueries } from "@/lib/auth";
import { randomSaying } from "@/lib/cowsay";
import { ServerUnreachable } from "@/routes/_auth/-auth-page";

export const Route = createFileRoute("/_auth")({
  // A 401 is the signed-out answer. Any other failure means that the server cannot be reached.
  beforeLoad: async ({ context }) => ({
    signedInUser: await context.queryClient
      .query({ ...authQueries.me(), staleTime: 0 })
      .catch((error: unknown) => {
        if (isApiError(error) && error.status === 401) return undefined;
        throw error;
      }),
  }),

  onEnter: ({ context }) => context.queryClient.clear(),

  component: () => (
    <AuthFrame>
      <Outlet />
    </AuthFrame>
  ),
  errorComponent: (props) => (
    <AuthFrame>
      <ServerUnreachable {...props} />
    </AuthFrame>
  ),
});

function AuthFrame({ children }: { children: ReactNode }) {
  const [saying] = useState(randomSaying);

  return (
    <div className="-mt-(--connection-banner-height) flex min-h-svh flex-col items-center justify-center p-4 sm:p-6 md:p-10">
      <img
        src={bannerUrl}
        alt={m.sign_in_banner_alt()}
        className="mb-6 h-auto w-64 sm:mb-8 sm:w-80 md:w-96"
      />

      <div className="w-full max-w-sm md:max-w-4xl">
        <Card className="overflow-hidden p-0">
          <CardContent className="grid grid-cols-1 p-0 md:grid-cols-2">
            <div className="flex flex-col justify-center">{children}</div>

            {/* On a phone, a short row under the form, so the form stays on the first screen. */}
            <div className="flex items-center gap-4 bg-muted px-5 py-3 md:min-h-105 md:flex-col md:justify-center md:gap-10 md:px-7 md:py-10 md:text-center">
              <img
                src={paronUrl}
                alt=""
                className="aspect-square w-14 shrink-0 md:w-3/5 md:max-w-50"
              />
              <p className="text-sm text-balance md:min-h-22 md:max-w-80 md:text-lg">
                {/* eslint-disable-next-line i18next/no-literal-string */}
                <span className="font-mono text-warning md:text-base">
                  [¬_¬]
                </span>{" "}
                <span className="font-bold italic">{saying}</span>
              </p>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
