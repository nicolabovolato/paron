import type {
  AuditEventResponse,
  AuditMachine,
  AuditProtocols,
  AuditSsh,
  AuditUser,
  AuthenticationMethod,
  Protocol,
} from "@/generated/api/types.gen";
import { zAuthenticationMethod, zProtocol } from "@/generated/api/zod.gen";
import { m } from "@/generated/paraglide/messages";
import { formatDateTime } from "@/lib/format";
import { PROTOCOL_NAMES, protocolName } from "@/lib/protocols";
import { ROLES } from "@/lib/roles";

export type DetailRow = {
  key: string;
  label: string;
  before?: string;
  after?: string;
  monospace?: boolean;
};

type EventPayload<Type extends AuditEventResponse["type"]> = Extract<
  AuditEventResponse,
  { type: Type }
>["payload"];

export function createdUserRows({ user }: EventPayload<"user.created">) {
  return differenceRows([], userFields(user));
}

export function changedUserRows({
  before,
  after,
}: EventPayload<"user.updated">) {
  return differenceRows(userFields(before), userFields(after));
}

const AUTH_METHOD_NAMES = {
  password: m.audit_method_password,
  totp: m.audit_method_totp,
  recovery_code: m.audit_method_recovery_code,
  passkey: m.audit_method_passkey,
} satisfies Record<AuthenticationMethod, () => string>;

// An entry keeps the name of a method that a later release can remove.
export function authMethodName(method: string) {
  return zAuthenticationMethod.validate(method)
    ? AUTH_METHOD_NAMES[method]()
    : method;
}

export function signedInRows({ methods }: EventPayload<"user.signed_in">) {
  return [
    {
      key: "methods",
      label: m.audit_field_sign_in_methods(),
      after: methods.map(authMethodName).join(", "),
    },
  ] satisfies DetailRow[];
}

export function recoveryStartedRows({
  expires_at,
}: EventPayload<"user.recovery_started">) {
  return [
    {
      key: "recovery_expiry",
      label: m.audit_field_recovery_expiry(),
      after: formatDateTime(expires_at),
    },
  ] satisfies DetailRow[];
}

export function createdMachineRows({
  machine,
}: EventPayload<"machine.created">) {
  return differenceRows([], machineFields(machine));
}

export function changedMachineRows({
  before,
  after,
  credentials_replaced,
}: EventPayload<"machine.updated">) {
  return [
    ...differenceRows(machineFields(before), machineFields(after)),
    ...credentials_replaced.map((protocol) => ({
      key: `${protocol}.credentials`,
      label: protocolFieldLabel(protocol, m.audit_field_credentials()),
      after: m.audit_value_replaced(),
    })),
  ] satisfies DetailRow[];
}

type Field = { key: string; label: string; value: string; monospace?: boolean };

function differenceRows(before: Field[], after: Field[]) {
  const keys = [...new Set([...before, ...after].map(({ key }) => key))];

  return keys.flatMap((key) => {
    const beforeField = before.find((candidate) => candidate.key === key);
    const afterField = after.find((candidate) => candidate.key === key);
    const field = beforeField ?? afterField;

    return !field || beforeField?.value === afterField?.value
      ? []
      : [
          {
            key,
            label: field.label,
            before: beforeField?.value,
            after: afterField?.value,
            monospace: field.monospace,
          } satisfies DetailRow,
        ];
  });
}

function userRoleLabel(role: string) {
  for (const [known, label] of ROLES) if (known === role) return label();
  return role;
}

function userFields(user: AuditUser) {
  return [
    {
      key: "username",
      label: m.user_username_label(),
      value: user.username,
      monospace: true,
    },
    {
      key: "display_name",
      label: m.user_display_name_label(),
      value: user.display_name,
    },
    {
      key: "roles",
      label: m.roles_title(),
      value: user.roles.map(userRoleLabel).join(", ") || m.audit_value_none(),
    },
    {
      key: "status",
      label: m.audit_field_status(),
      value: {
        active: m.user_status_active,
        suspended: m.user_status_suspended,
      }[user.status](),
    },
  ] satisfies Field[];
}

function machineFields({ name, connectivity, protocols }: AuditMachine) {
  return [
    { key: "name", label: m.machine_field_name(), value: name },
    {
      key: "connection",
      label: m.machine_field_connection(),
      value: {
        direct: m.machine_connectivity_direct,
        tunneled: m.machine_connectivity_tunneled,
      }[connectivity.kind](),
    },
    ...(connectivity.kind === "direct"
      ? [
          {
            key: "host",
            label: m.machine_field_host(),
            value: connectivity.host,
            monospace: true,
          },
        ]
      : [
          {
            key: "tunnel",
            label: m.audit_field_tunnel(),
            value: {
              not_enrolled: m.tunnel_state_not_set_up,
              invited: m.tunnel_state_pending,
              enrolled: m.tunnel_state_registered,
            }[connectivity.enrollment.state](),
          },
          ...(connectivity.enrollment.state === "invited"
            ? [
                {
                  key: "token_expiry",
                  label: m.audit_field_token_expiry(),
                  value: formatDateTime(connectivity.enrollment.expires_at),
                },
              ]
            : []),
        ]),
    ...protocolsFields(protocols),
  ] satisfies Field[];
}

type ProtocolFields = {
  [P in Protocol]: (settings: NonNullable<AuditProtocols[P]>) => Field[];
};

const PROTOCOL_FIELDS: ProtocolFields = { ssh: sshFields };

function protocolsFields(protocols: AuditProtocols) {
  const present = zProtocol.options.filter((protocol) => protocols[protocol]);

  return [
    {
      key: "protocols",
      label: m.audit_field_protocols(),
      value:
        present.map((protocol) => PROTOCOL_NAMES[protocol]).join(", ") ||
        m.audit_value_none(),
    },
    ...present.flatMap((protocol) => {
      const settings = protocols[protocol];
      return settings ? protocolFields(protocol, settings) : [];
    }),
  ] satisfies Field[];
}

function protocolFields<P extends Protocol>(
  protocol: P,
  settings: NonNullable<AuditProtocols[P]>,
) {
  return PROTOCOL_FIELDS[protocol](settings).map((field) => ({
    ...field,
    key: `${protocol}.${field.key}`,
    label: protocolFieldLabel(protocol, field.label),
  }));
}

function protocolFieldLabel(recorded: string, field: string) {
  return m.audit_field_of_protocol({ protocol: protocolName(recorded), field });
}

function sshFields(ssh: AuditSsh) {
  return [
    {
      key: "user",
      label: m.machine_field_user(),
      value: ssh.user,
      monospace: true,
    },
    {
      key: "port",
      label: m.machine_field_port(),
      value: String(ssh.port),
      monospace: true,
    },
    {
      key: "authentication",
      label: m.machine_field_authentication(),
      value: {
        password: m.machine_authentication_password,
        managed_keypair: m.machine_authentication_managed_key,
        imported_keypair: m.machine_authentication_imported_key,
      }[ssh.authentication](),
    },
    {
      key: "host_key",
      label: m.machine_field_host_key(),
      value: ssh.host_key_fingerprint,
      monospace: true,
    },
  ] satisfies Field[];
}
