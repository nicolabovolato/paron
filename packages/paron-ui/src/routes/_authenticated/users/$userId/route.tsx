import { createFileRoute } from "@tanstack/react-router";

import { userQueries } from "@/features/users/lib/api";
import { zReadUserPath } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";

export const Route = createFileRoute("/_authenticated/users/$userId")({
  staticData: {
    notFound: {
      title: m.user_not_found_title,
      description: m.user_not_found_description,
    },
  },
  params: {
    parse: ({ userId }) => ({ userId: zReadUserPath.shape.id.parse(userId) }),
  },
  loader: async ({ context, params }) => {
    const user = await context.queryClient.query({
      ...userQueries.detail(params.userId),
      staleTime: "static",
    });

    return { breadcrumb: user.display_name };
  },
});
