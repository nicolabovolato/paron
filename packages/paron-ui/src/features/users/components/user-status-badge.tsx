import { CircleCheck, TriangleAlert } from "lucide-react";

import { Badge } from "@/components/shadcn/badge";
import type { UserStatus } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function UserStatusBadge({ status }: { status: UserStatus }) {
  return status === "active" ? (
    <Badge variant="outline" className="text-success">
      <CircleCheck />
      {m.user_status_active()}
    </Badge>
  ) : (
    <Badge variant="outline" className="text-warning">
      <TriangleAlert />
      {m.user_status_suspended()}
    </Badge>
  );
}
