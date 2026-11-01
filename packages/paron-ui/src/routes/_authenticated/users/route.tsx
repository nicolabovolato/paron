import { createFileRoute } from "@tanstack/react-router";

import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/users")({
  staticData: { permission: "user:read" },
  loader: () => ({ breadcrumb: m.users_crumb() }),
});
