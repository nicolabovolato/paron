import { createFileRoute } from "@tanstack/react-router";

import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/machines")({
  staticData: { permission: "machine:read" },
  loader: () => ({ breadcrumb: m.machines_crumb() }),
});
