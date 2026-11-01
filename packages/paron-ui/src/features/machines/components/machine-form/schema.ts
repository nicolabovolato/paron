import { z } from "zod";

import type {
  MachineRequest,
  MachineResponse,
  PartialSshAuthenticationRequest,
  PartialSshRequest,
  PartialUpdateMachineRequest,
  SshRequest,
} from "@/generated/api/types.gen";
import {
  zConnectTimeout,
  zHost,
  zMachineName,
  zPort,
  zPrivateKey,
  zPublicKey,
  zSshPassword,
  zSshUser,
} from "@/generated/api/zod.gen";

// Adds the issues of a generated schema to a field, as they are, so the field
// words them like any other.
function checkField(
  context: z.RefinementCtx,
  path: string[],
  value: string,
  schema: z.ZodType<string>,
) {
  for (const issue of schema.safeParse(value).error?.issues ?? [])
    // Zod's types only take a new issue as "custom"; this one keeps its code.
    context.issues.push({
      ...issue,
      input: value,
      path: [...path, ...issue.path],
    } as z.core.$ZodRawIssue);
}

const connectivitySchema = z
  .object({ kind: z.enum(["direct", "tunneled"]), host: z.string().trim() })
  .superRefine((connectivity, context) => {
    if (connectivity.kind === "direct")
      checkField(context, ["host"], connectivity.host, zHost);
  });

const authenticationMethodSchema = z
  .object({
    replace: z.boolean(),
    kind: z.enum(["keypair", "password"]),
    password: z.string(),
    source: z.object({
      kind: z.enum(["managed", "imported"]),
      private_key: z.string().trim(),
    }),
  })
  .superRefine((method, context) => {
    if (!method.replace) return;
    if (method.kind === "password")
      checkField(context, ["password"], method.password, zSshPassword);
    if (method.kind === "keypair" && method.source.kind === "imported")
      checkField(
        context,
        ["source", "private_key"],
        method.source.private_key,
        zPrivateKey,
      );
  });

const authenticationSchema = z
  .object({ host_key: z.string(), method: authenticationMethodSchema })
  .superRefine((authentication, context) =>
    checkField(context, ["host_key"], authentication.host_key, zPublicKey),
  );

const sshSchema = z.object({
  user: z.string().trim().pipe(zSshUser),
  port: zPort,
  connect_timeout_seconds: zConnectTimeout,
  authentication: authenticationSchema,
});

// Shaped like MachineRequest, so a validation error the server reports at
// `protocols.ssh.user` lands on the field named `protocols.ssh.user`.
export const machineFormSchema = z.object({
  name: z.string().trim().pipe(zMachineName),
  connectivity: connectivitySchema,
  protocols: z.object({ ssh: sshSchema }),
});

export type MachineFormValues = z.infer<typeof machineFormSchema>;

type SshFormValues = MachineFormValues["protocols"]["ssh"];

export function initialValues(machine?: MachineResponse) {
  const ssh = machine?.protocols.ssh;
  const authentication = ssh?.authentication;
  const method = authentication?.method;

  return {
    name: machine?.name ?? "",
    connectivity: {
      kind: machine?.connectivity.kind ?? "direct",
      host:
        machine?.connectivity.kind === "direct"
          ? machine.connectivity.host
          : "",
    },
    protocols: {
      ssh: {
        user: ssh?.user ?? "",
        port: ssh?.port ?? 22,
        connect_timeout_seconds: ssh?.connect_timeout_seconds ?? 10,
        authentication: {
          host_key: authentication?.host_key ?? "",
          method: {
            replace: method === undefined,
            kind: method?.kind ?? "keypair",
            password: "",
            source: {
              kind: method?.kind === "keypair" ? method.source : "managed",
              private_key: "",
            },
          },
        },
      },
    },
  } satisfies MachineFormValues;
}

export function toCreateRequest(values: MachineFormValues) {
  const { name, connectivity, protocols } = machineFormSchema.parse(values);

  return {
    name,
    connectivity: toConnectivityRequest(connectivity),
    protocols: { ssh: toSshRequest(protocols.ssh) },
  } satisfies MachineRequest;
}

export function toUpdateRequest(
  typed: MachineFormValues,
  machine: MachineResponse,
) {
  const values = machineFormSchema.parse(typed);
  const saved = initialValues(machine);
  const connectivityChanged =
    values.connectivity.kind !== saved.connectivity.kind ||
    values.connectivity.host !== saved.connectivity.host;
  const ssh = machine.protocols.ssh
    ? toSshUpdateRequest(values.protocols.ssh, saved.protocols.ssh)
    : toSshRequest(values.protocols.ssh);

  return {
    name: ifChanged(values.name, saved.name),
    connectivity: connectivityChanged
      ? toConnectivityRequest(values.connectivity)
      : undefined,
    protocols: ssh === undefined ? undefined : { ssh },
  } satisfies PartialUpdateMachineRequest;
}

function toSshRequest(ssh: SshFormValues) {
  return {
    user: ssh.user,
    port: ssh.port,
    connect_timeout_seconds: ssh.connect_timeout_seconds,
    authentication: {
      host_key: ssh.authentication.host_key,
      method: toMethodRequest(ssh.authentication.method),
    },
  } satisfies SshRequest;
}

function toSshUpdateRequest(ssh: SshFormValues, saved: SshFormValues) {
  const authentication = {
    host_key: ifChanged(
      ssh.authentication.host_key,
      saved.authentication.host_key,
    ),
    method: ssh.authentication.method.replace
      ? toMethodRequest(ssh.authentication.method)
      : undefined,
  } satisfies PartialSshAuthenticationRequest;

  const request = {
    user: ifChanged(ssh.user, saved.user),
    port: ifChanged(ssh.port, saved.port),
    connect_timeout_seconds: ifChanged(
      ssh.connect_timeout_seconds,
      saved.connect_timeout_seconds,
    ),
    authentication: anyValue(authentication) ? authentication : undefined,
  } satisfies PartialSshRequest;

  return anyValue(request) ? request : undefined;
}

function ifChanged<T>(value: T, saved: T) {
  return value !== saved ? value : undefined;
}

function anyValue(request: object) {
  return Object.values(request).some((value) => value !== undefined);
}

function toConnectivityRequest(
  connectivity: MachineFormValues["connectivity"],
) {
  return connectivity.kind === "direct"
    ? ({ kind: "direct", host: connectivity.host } as const)
    : ({ kind: "tunneled" } as const);
}

function toMethodRequest(method: SshFormValues["authentication"]["method"]) {
  return method.kind === "password"
    ? ({ kind: "password", password: method.password } as const)
    : ({
        kind: "keypair",
        source:
          method.source.kind === "managed"
            ? ({ kind: "managed", algorithm: "ed25519" } as const)
            : ({
                kind: "imported",
                private_key: method.source.private_key,
              } as const),
      } as const);
}
