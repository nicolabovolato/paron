import type { ComponentProps } from "react";

import { Avatar, AvatarFallback } from "@/components/shadcn/avatar";
import type { UserResponse } from "@/generated/api/types.gen";

export function UserAvatar({
  user,
  size,
}: {
  user: UserResponse;
  size?: ComponentProps<typeof Avatar>["size"];
}) {
  const initials = user.display_name
    .split(/[\s._-]+/)
    .filter(Boolean)
    .slice(0, 2)
    .map((word) => word.charAt(0).toUpperCase())
    .join("");

  return (
    <Avatar size={size}>
      <AvatarFallback>{initials}</AvatarFallback>
    </Avatar>
  );
}
