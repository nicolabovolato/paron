import { http, HttpResponse } from "msw";

import type {
  ApiError,
  AuditEntryResponse,
  AuditPageResponse,
  CredentialsResponse,
  MachineResponse,
  PageResponseMachineResponse,
  PageResponseUserResponse,
  Permission,
  RecoveryOptionsResponse,
  StartedSignInResponse,
  UserResponse,
} from "@/generated/api/types.gen";
import { zPermission } from "@/generated/api/zod.gen";

export const ADMINISTRATOR = {
  id: "018f0000-0000-7000-8000-000000000000",
  username: "admin",
  display_name: "Admin",
  status: "active",
  roles: ["administrator"],
  permissions: zPermission.options,
  created_at: "2026-01-01T00:00:00Z",
} satisfies UserResponse;

export const administratorWithout = (permission: Permission) =>
  ({
    ...ADMINISTRATOR,
    permissions: ADMINISTRATOR.permissions.filter(
      (grantedPermission) => grantedPermission !== permission,
    ),
  }) satisfies UserResponse;

export const OPERATOR = {
  id: "018f0000-0000-7000-8000-000000000002",
  username: "giulia.r",
  display_name: "Giulia Rossi",
  status: "active",
  roles: ["operator"],
  permissions: [
    "machine:read",
    "machine:write",
    "machine:delete",
    "machine:operate",
    "machine:enroll",
  ],
  created_at: "2026-01-02T00:00:00Z",
} satisfies UserResponse;

export const MACHINE = {
  id: "018f0000-0000-7000-8000-000000000001",
  name: "forno-01",
  connectivity: { kind: "direct", host: "10.0.0.5" },
  protocols: {
    ssh: {
      user: "deploy",
      port: 22,
      connect_timeout_seconds: 10,
      authentication: {
        host_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleHostKey",
        method: {
          kind: "keypair",
          source: "managed",
          public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleClientKey",
        },
      },
    },
  },
  capabilities: {
    connection_test: [{ kind: "protocol", protocol: "ssh" }],
    terminal: [{ kind: "protocol", protocol: "ssh" }],
  },
} satisfies MachineResponse;

export const EMPTY_PAGE = {
  items: [],
  total: 0,
} satisfies PageResponseUserResponse & PageResponseMachineResponse;

export const MACHINE_UPDATED = {
  id: "018f0000-0000-7000-8000-0000000000a1",
  created_at: "2026-10-08T12:00:00.000Z",
  actor: { type: "user", id: ADMINISTRATOR.id, name: "admin" },
  target: { type: "machine", id: MACHINE.id, name: "web-01" },
  event: {
    type: "machine.updated",
    payload: {
      before: {
        name: "web-01",
        connectivity: { kind: "direct", host: "10.0.0.5" },
        protocols: {
          ssh: {
            user: "root",
            port: 22,
            host_key_fingerprint: "SHA256:abc",
            authentication: "password",
          },
        },
      },
      after: {
        name: "web-01",
        connectivity: { kind: "direct", host: "10.0.0.6" },
        protocols: {
          ssh: {
            user: "root",
            port: 22,
            host_key_fingerprint: "SHA256:abc",
            authentication: "password",
          },
        },
      },
      credentials_replaced: [],
    },
  },
} satisfies AuditEntryResponse;

export const signedInAsAdministrator = http.get("/api/auth/session", () =>
  HttpResponse.json(ADMINISTRATOR),
);

export const startedSignIn = http.post("/api/auth/sign-in", () =>
  HttpResponse.json({
    accepted: [{ kind: "password" }],
  } satisfies StartedSignInResponse),
);

export const recoveryOptions = http.get("/api/auth/recovery", () =>
  HttpResponse.json({
    methods: ["password", "passkey"],
    expires_at: "2026-10-24T12:00:00Z",
  } satisfies RecoveryOptionsResponse),
);

export const PASSWORD_ONLY = {
  password: { updated_at: "2026-01-01T00:00:00Z" },
  passkeys: [],
} satisfies CredentialsResponse;

// What the overview reads on a new installation: one administrator with a password, nothing else.
export const freshInstall = [
  http.get("/api/account/credentials", () => HttpResponse.json(PASSWORD_ONLY)),
  http.get("/api/machines", () => HttpResponse.json(EMPTY_PAGE)),
  http.get("/api/audit", () =>
    HttpResponse.json({ items: [] } satisfies AuditPageResponse),
  ),
  http.get("/api/users", () =>
    HttpResponse.json({ items: [ADMINISTRATOR], total: 1 }),
  ),
];

export const errorResponse = (error: ApiError) =>
  HttpResponse.json(error, { status: error.status });

export const noSessionResponse = () =>
  errorResponse({ status: 401, detail: "no session" });
