import { createFileRoute, redirect, useNavigate } from "@tanstack/react-router";
import { CircleCheck, Clock } from "lucide-react";
import { z } from "zod";

import { Alert, AlertDescription } from "@/components/shadcn/alert";
import { SignIn } from "@/components/sign-in";
import { m } from "@/generated/paraglide/messages";
import { PageTitle } from "@/routes/_auth/-auth-page";

export const Route = createFileRoute("/_auth/login")({
  validateSearch: z.object({
    redirect: z.string().optional().catch(undefined),
    expired: z.boolean().optional().catch(undefined),
    recovered: z.enum(["password", "passkey"]).optional().catch(undefined),
  }),

  beforeLoad: ({ context, search }) => {
    if (context.signedInUser) throw redirect({ to: search.redirect ?? "/" });
  },

  component: RouteComponent,
});

function RouteComponent() {
  const navigate = useNavigate();
  const search = Route.useSearch();

  return (
    <div className="p-6 md:p-8">
      <title>{m.page_title({ page: m.login_page_title() })}</title>
      <SignIn
        onSignedIn={() =>
          navigate({ to: search.redirect ?? "/", replace: true })
        }
        heading={
          <PageTitle
            title={m.login_title()}
            description={m.login_description()}
          />
        }
        nextStepHeading={
          <PageTitle
            title={m.second_factor_title()}
            description={m.second_factor_description()}
          />
        }
        notice={
          <>
            {search.expired && (
              <Alert>
                <Clock />
                <AlertDescription>{m.login_session_expired()}</AlertDescription>
              </Alert>
            )}
            {search.recovered && (
              <Alert>
                <CircleCheck />
                <AlertDescription>
                  {{
                    password: m.login_recovered_password,
                    passkey: m.login_recovered_passkey,
                  }[search.recovered]()}
                </AlertDescription>
              </Alert>
            )}
          </>
        }
      />
    </div>
  );
}
