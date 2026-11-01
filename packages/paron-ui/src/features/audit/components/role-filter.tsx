import { ToggleGroup, ToggleGroupItem } from "@/components/shadcn/toggle-group";
import type { AuditRole } from "@/features/audit/lib/api";
import { m } from "@/generated/paraglide/messages";

const ROLES = [
  ["actor_or_target", m.audit_role_actor_or_target],
  ["actor", m.audit_role_actor],
  ["target", m.audit_role_target],
] as const satisfies ReadonlyArray<readonly [AuditRole, () => string]>;

export function RoleFilter({
  role,
  onChange,
}: {
  role: AuditRole;
  onChange: (role: AuditRole) => void;
}) {
  return (
    <ToggleGroup
      aria-label={m.audit_role_label()}
      variant="outline"
      size="sm"
      spacing={0}
      value={[role]}
      onValueChange={([pressed]) => {
        const [pressedRole] = ROLES.find(([known]) => known === pressed) ?? [];
        if (pressedRole) onChange(pressedRole);
      }}
    >
      {ROLES.map(([value, label]) => (
        <ToggleGroupItem key={value} value={value}>
          {label()}
        </ToggleGroupItem>
      ))}
    </ToggleGroup>
  );
}
