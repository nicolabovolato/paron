import { ParaglideMessage } from "@inlang/paraglide-js-react";

import { CodeBlock } from "@/components/code-block";
import { DialogContent } from "@/components/dialog";
import { RelativeTime } from "@/components/relative-time";
import { Button } from "@/components/shadcn/button";
import {
  DialogClose,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/shadcn/dialog";
import type { UserResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function RecoveryLinkDialog({
  user,
  url,
  expiresAt,
}: {
  user: UserResponse;
  url: string;
  expiresAt: string;
}) {
  return (
    <DialogContent className="sm:max-w-xl" showCloseButton={false}>
      <DialogHeader>
        <DialogTitle>
          {m.user_recovery_link_title({ username: user.username })}
        </DialogTitle>
        <DialogDescription>
          <ParaglideMessage
            message={m.user_recovery_link_description}
            inputs={{ name: user.display_name }}
            markup={{ expiry: () => <RelativeTime iso={expiresAt} /> }}
          />
        </DialogDescription>
      </DialogHeader>

      <CodeBlock value={url} copyable />

      <DialogFooter>
        <DialogClose
          render={
            <Button
              variant="ghost"
              className="bg-warning/10 text-warning hover:bg-warning/20 hover:text-warning dark:hover:bg-warning/20"
            />
          }
        >
          {m.user_recovery_link_done()}
        </DialogClose>
      </DialogFooter>
    </DialogContent>
  );
}
