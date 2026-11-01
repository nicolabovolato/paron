import type { EnrollmentResponse } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export function isExpired(enrollment: EnrollmentResponse, now: number) {
  return (
    enrollment.state === "invited" &&
    new Date(enrollment.expires_at).getTime() <= now
  );
}

export function testConnectionDisabled({
  tunneled,
  enrolled,
}: {
  tunneled: boolean;
  enrolled: boolean;
}) {
  if (tunneled && !enrolled) {
    return {
      reason: m.connection_test_disabled_not_enrolled(),
    };
  }

  return false as const;
}
