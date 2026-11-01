import type { Role } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";

export const ROLES = [
  ["administrator", m.roles_administrator, m.roles_administrator_description],
  ["operator", m.roles_operator, m.roles_operator_description],
] as const satisfies ReadonlyArray<readonly [Role, () => string, () => string]>;
