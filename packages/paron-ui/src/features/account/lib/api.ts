import { useMutation } from "@tanstack/react-query";

import { useReauthentication } from "@/components/sign-in";
import { updateCredentialsMutation } from "@/generated/api/@tanstack/react-query.gen";

export function useUpdateCredentials() {
  const withReauthentication = useReauthentication();
  return useMutation(withReauthentication(updateCredentialsMutation()));
}

/** For a start: it changes nothing until its confirmation. */
export function useStartCredentialAction() {
  const withReauthentication = useReauthentication();
  return useMutation(
    withReauthentication({
      ...updateCredentialsMutation(),
      meta: { invalidates: false },
    }),
  );
}
