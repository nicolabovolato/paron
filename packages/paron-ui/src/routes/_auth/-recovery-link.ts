import { useRouter } from "@tanstack/react-router";

import type { RecoveryLinkResponse } from "@/generated/api/types.gen";

export function useRecoveryUrl() {
  const router = useRouter();

  return ({ token, public_origin }: RecoveryLinkResponse) =>
    new URL(
      router.buildLocation({
        to: "/recover-account",
        hash: new URLSearchParams({ token }).toString(),
      }).href,
      public_origin,
    ).href;
}

export function recoveryToken(hash: string) {
  return new URLSearchParams(hash).get("token");
}
