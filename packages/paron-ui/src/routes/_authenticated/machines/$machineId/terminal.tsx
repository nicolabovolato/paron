import { useSuspenseQuery } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { z } from "zod";

import { MachineTerminal } from "@/features/machines/components/terminal";
import { machineQueries } from "@/features/machines/lib/api";
import { zProtocol } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute(
  "/_authenticated/machines/$machineId/terminal",
)({
  staticData: {
    permission: "machine:operate",
    width: "full",
    height: "screen",
  },
  validateSearch: z.object({ protocol: zProtocol }),
  loader: () => ({ breadcrumb: m.terminal_crumb() }),
  component: RouteComponent,
});

function RouteComponent() {
  const { machineId } = Route.useParams();
  const { protocol } = Route.useSearch();
  const { data: machine } = useSuspenseQuery(machineQueries.detail(machineId));

  return <MachineTerminal machine={machine} protocol={protocol} />;
}
