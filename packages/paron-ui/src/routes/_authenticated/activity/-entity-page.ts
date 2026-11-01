import { linkOptions, useRouter } from "@tanstack/react-router";

import type { AuditEntityResponse } from "@/generated/api/types.gen";
import { usePermissions } from "@/hooks/permissions";

export function useEntityPage() {
  const router = useRouter();
  const permissions = usePermissions();

  return ({ type, id }: AuditEntityResponse) => {
    const page =
      type === "user"
        ? linkOptions({ to: "/users/$userId", params: { userId: id } })
        : linkOptions({
            to: "/machines/$machineId",
            params: { machineId: id },
          });

    const allowed = router
      .matchRoutes(router.buildLocation(page).pathname)
      .every(
        ({ staticData }) =>
          !staticData.permission || permissions.has(staticData.permission),
      );

    return allowed ? page : undefined;
  };
}
