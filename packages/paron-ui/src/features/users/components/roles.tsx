import { useId } from "react";

import { Badge } from "@/components/shadcn/badge";
import { Checkbox } from "@/components/shadcn/checkbox";
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldGroup,
  FieldLabel,
  FieldLegend,
  FieldSet,
} from "@/components/shadcn/field";
import type { Role } from "@/generated/api/types.gen";
import { m } from "@/generated/paraglide/messages";
import { ROLES } from "@/lib/roles";

export function RolesField({
  value,
  disabled = false,
  onChange,
}: {
  value: Array<Role>;
  disabled?: boolean;
  onChange: (roles: Array<Role>) => void;
}) {
  const id = useId();
  const administrator = value.includes("administrator");

  return (
    <FieldSet>
      <FieldLegend variant="label">{m.roles_title()}</FieldLegend>
      <FieldGroup className="gap-3">
        {ROLES.map(([role, label, description]) => {
          const included = role !== "administrator" && administrator;

          return (
            <Field
              key={role}
              orientation="horizontal"
              data-disabled={disabled || included}
            >
              <Checkbox
                id={`${id}-${role}`}
                checked={value.includes(role) && !included}
                disabled={disabled || included}
                onCheckedChange={(checked) => {
                  if (role === "administrator")
                    onChange(checked ? ["administrator"] : []);
                  else
                    onChange(
                      checked
                        ? [...value, role]
                        : value.filter((other) => other !== role),
                    );
                }}
              />
              <FieldContent>
                <FieldLabel htmlFor={`${id}-${role}`}>{label()}</FieldLabel>
                <FieldDescription>
                  {included
                    ? m.roles_included_in_administrator()
                    : description()}
                </FieldDescription>
              </FieldContent>
            </Field>
          );
        })}
      </FieldGroup>
    </FieldSet>
  );
}

export function RoleList({ roles }: { roles: Array<Role> }) {
  return (
    <dl className="grid grid-cols-1 gap-3 text-sm sm:grid-cols-[max-content_1fr] sm:items-baseline">
      {ROLES.filter(([role]) => roles.includes(role)).map(
        ([role, label, description]) => (
          <div
            key={role}
            className="flex flex-col items-start gap-1 sm:contents"
          >
            <dt>
              <Badge variant="secondary">{label()}</Badge>
            </dt>
            <dd className="text-muted-foreground">{description()}</dd>
          </div>
        ),
      )}
    </dl>
  );
}

export function RoleBadges({ roles }: { roles: Array<Role> }) {
  return (
    <div className="flex flex-wrap gap-1">
      {ROLES.filter(([role]) => roles.includes(role)).map(([role, label]) => (
        <Badge key={role} variant="secondary">
          {label()}
        </Badge>
      ))}
    </div>
  );
}
