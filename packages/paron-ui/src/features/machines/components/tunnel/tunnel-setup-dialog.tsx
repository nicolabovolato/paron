import { ParaglideMessage } from "@inlang/paraglide-js-react";

import { CodeBlock, CopyButton } from "@/components/code-block";
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
import type {
  EnrollmentInvitationResponse,
  MachineResponse,
} from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function TunnelSetupDialog({
  machine,
  token,
}: {
  machine: MachineResponse;
  token: EnrollmentInvitationResponse;
}) {
  return (
    <DialogContent className="sm:max-w-2xl" showCloseButton={false}>
      <DialogHeader>
        <DialogTitle>
          {m.tunnel_setup_title({ name: machine.name })}
        </DialogTitle>
        <DialogDescription>
          <ParaglideMessage
            message={m.tunnel_setup_description}
            markup={{ expiry: () => <RelativeTime iso={token.expires_at} /> }}
          />
        </DialogDescription>
      </DialogHeader>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <p className="text-sm text-muted-foreground">
            {m.tunnel_setup_command_intro({ name: machine.name })}
          </p>
          <CopyButton value={token.token} label={m.tunnel_setup_copy_token()} />
        </div>
        <CodeBlock
          value={`paron tunnel bootstrap \\\n  --token '${token.token}'`}
          copyable
        />
      </div>

      <DialogFooter>
        <DialogClose
          render={
            <Button
              variant="ghost"
              className="bg-warning/10 text-warning hover:bg-warning/20 hover:text-warning dark:hover:bg-warning/20"
            />
          }
        >
          {m.tunnel_setup_done()}
        </DialogClose>
      </DialogFooter>
    </DialogContent>
  );
}
