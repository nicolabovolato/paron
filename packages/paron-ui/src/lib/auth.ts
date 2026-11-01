import { useMutation } from "@tanstack/react-query";

import {
  ownCredentialsOptions,
  readSessionOptions,
  signOutMutation,
} from "@/generated/api/@tanstack/react-query.gen";

export const authQueries = {
  me: () => readSessionOptions(),
  credentials: () => ownCredentialsOptions(),
};

export function useSignOut() {
  return useMutation({
    ...signOutMutation(),
    // Signed out, every query would fail.
    meta: { invalidates: false },
  });
}
