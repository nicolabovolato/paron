import { useSuspenseQuery } from "@tanstack/react-query";

import type { ReadSessionResponse } from "@/generated/api/types.gen";
import { authQueries } from "@/lib/auth";

export function usePermissions() {
  return useSuspenseQuery({
    ...authQueries.me(),
    select: (me: ReadSessionResponse) => new Set(me.permissions),
  }).data;
}

export class ForbiddenError extends Error {}
