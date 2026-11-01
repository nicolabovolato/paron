import { cn } from "cn";

import { UserAvatar } from "@/features/users/components/user-avatar";
import { UserStatusBadge } from "@/features/users/components/user-status-badge";
import type { UserResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function UserBadge({
  user,
  isMe = false,
  showStatus = false,
  size = "default",
}: {
  user: UserResponse;
  isMe?: boolean;
  showStatus?: boolean;
  size?: "default" | "lg";
}) {
  const Name = size === "lg" ? "h1" : "span";

  return (
    <div
      className={cn(
        "flex min-w-0 flex-1 items-center text-left",
        size === "lg" ? "gap-3" : "gap-2.5 text-sm",
      )}
    >
      <UserAvatar user={user} size={size} />
      <div
        className={cn(
          "flex min-w-0 flex-col",
          size === "lg" ? "gap-0.5" : "leading-tight",
        )}
      >
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <Name
            className={cn(
              "truncate",
              size === "lg"
                ? "text-xl font-semibold tracking-tight"
                : "font-medium",
            )}
          >
            {user.display_name}
            {isMe && (
              <span className="font-normal text-muted-foreground">
                {" "}
                {m.user_badge_you()}
              </span>
            )}
          </Name>
          {showStatus && user.status !== "active" && (
            <UserStatusBadge status={user.status} />
          )}
        </div>
        <span
          className={cn(
            "truncate text-muted-foreground",
            size === "lg" ? "text-sm" : "text-xs",
          )}
        >
          {user.username}
        </span>
      </div>
    </div>
  );
}
