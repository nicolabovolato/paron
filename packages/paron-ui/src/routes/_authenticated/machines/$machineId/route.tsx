import { createFileRoute } from "@tanstack/react-router";

import { machineQueries } from "@/features/machines/lib/api";
import { zReadMachinePath } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/machines/$machineId")({
  staticData: {
    notFound: {
      title: m.machine_not_found_title,
      description: m.machine_not_found_description,
    },
  },
  params: {
    parse: ({ machineId }) => ({
      machineId: zReadMachinePath.shape.id.parse(machineId),
    }),
  },
  loader: async ({ context, params }) => {
    const machine = await context.queryClient.query({
      ...machineQueries.detail(params.machineId),
      staleTime: "static",
    });

    return { breadcrumb: machine.name };
  },
});
