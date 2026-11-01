import { createFileRoute } from "@tanstack/react-router";

import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/activity")({
  loader: () => ({ breadcrumb: m.audit_crumb() }),
});
