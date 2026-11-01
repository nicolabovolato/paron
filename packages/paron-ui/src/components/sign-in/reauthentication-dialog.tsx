import { useSuspenseQuery } from "@tanstack/react-query";

import { DialogContent, useDialog } from "@/components/dialog";
import {
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/shadcn/dialog";
import { m } from "@/generated/paraglide/messages";
import { authQueries } from "@/lib/auth";

import { SignIn } from "./sign-in";

export function ReauthenticationDialog() {
  const dialog = useDialog();
  const { data: user } = useSuspenseQuery(authQueries.me());

  return (
    <DialogContent className="sm:max-w-100">
      <SignIn
        username={user.username}
        onSignedIn={dialog.close}
        heading={
          <DialogHeader>
            <DialogTitle>{m.reauthentication_title()}</DialogTitle>
            <DialogDescription>
              {m.reauthentication_description({ username: user.username })}
            </DialogDescription>
          </DialogHeader>
        }
        nextStepHeading={
          <DialogHeader>
            <DialogTitle>{m.second_factor_title()}</DialogTitle>
            <DialogDescription>
              {m.second_factor_description()}
            </DialogDescription>
          </DialogHeader>
        }
      />
    </DialogContent>
  );
}
