import { describe, expect, test } from "vitest";

import type { AuditMachine } from "@/generated/api/types.gen";
import { formatDateTime } from "@/lib/format";

import {
  changedMachineRows,
  createdMachineRows,
  createdUserRows,
} from "./details";

const WEB_01 = {
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
} satisfies AuditMachine;

describe("changedMachineRows()", () => {
  test("changed fields", () => {
    expect(
      changedMachineRows({
        before: WEB_01,
        after: {
          ...WEB_01,
          protocols: { ssh: { ...WEB_01.protocols.ssh, port: 2222 } },
        },
        credentials_replaced: ["ssh"],
      }),
    ).toEqual([
      {
        key: "ssh.port",
        label: "SSH - Port",
        before: "22",
        after: "2222",
        monospace: true,
      },
      { key: "ssh.credentials", label: "SSH - Credentials", after: "Replaced" },
    ]);
  });

  test("one-sided fields", () => {
    expect(
      changedMachineRows({
        before: WEB_01,
        after: {
          ...WEB_01,
          connectivity: {
            kind: "tunneled",
            enrollment: { state: "not_enrolled" },
          },
        },
        credentials_replaced: [],
      }),
    ).toEqual([
      {
        key: "connection",
        label: "Connection",
        before: "Direct",
        after: "Tunneled",
      },
      {
        key: "host",
        label: "Host",
        before: "10.0.0.5",
        after: undefined,
        monospace: true,
      },
      {
        key: "tunnel",
        label: "Tunnel",
        before: undefined,
        after: "Not set up",
      },
    ]);
  });

  test("removed SSH", () => {
    expect(
      changedMachineRows({
        before: WEB_01,
        after: { ...WEB_01, protocols: {} },
        credentials_replaced: [],
      }).map(({ key, after }) => ({ key, after })),
    ).toEqual([
      { key: "protocols", after: "None" },
      { key: "ssh.user", after: undefined },
      { key: "ssh.port", after: undefined },
      { key: "ssh.authentication", after: undefined },
      { key: "ssh.host_key", after: undefined },
    ]);
  });

  test("added SSH", () => {
    expect(
      changedMachineRows({
        before: { ...WEB_01, protocols: {} },
        after: WEB_01,
        credentials_replaced: [],
      }),
    ).toMatchObject([
      { key: "protocols", before: "None", after: "SSH" },
      { key: "ssh.user", before: undefined, after: "root" },
      { key: "ssh.port", before: undefined, after: "22" },
      { key: "ssh.authentication", before: undefined, after: "Password" },
      { key: "ssh.host_key", before: undefined, after: "SHA256:abc" },
    ]);
  });

  test("nothing changed", () => {
    expect(
      changedMachineRows({
        before: WEB_01,
        after: WEB_01,
        credentials_replaced: [],
      }),
    ).toEqual([]);
  });

  test("renewed token", () => {
    expect(
      changedMachineRows({
        before: {
          ...WEB_01,
          connectivity: {
            kind: "tunneled",
            enrollment: {
              state: "invited",
              expires_at: "2026-10-15T12:00:00Z",
            },
          },
        },
        after: {
          ...WEB_01,
          connectivity: {
            kind: "tunneled",
            enrollment: {
              state: "invited",
              expires_at: "2026-10-22T12:00:00Z",
            },
          },
        },
        credentials_replaced: [],
      }),
    ).toEqual([
      {
        key: "token_expiry",
        label: "Token expires",
        before: formatDateTime("2026-10-15T12:00:00Z"),
        after: formatDateTime("2026-10-22T12:00:00Z"),
      },
    ]);
  });
});

describe("createdMachineRows()", () => {
  test("no protocol", () => {
    expect(
      createdMachineRows({ machine: { ...WEB_01, protocols: {} } }).find(
        ({ key }) => key === "protocols",
      ),
    ).toEqual({ key: "protocols", label: "Protocols", after: "None" });
  });
});

describe("createdUserRows()", () => {
  test("first values", () => {
    expect(
      createdUserRows({
        user: {
          username: "giulia",
          display_name: "Giulia Rossi",
          roles: ["operator", "auditor"],
          status: "active",
        },
      }),
    ).toEqual([
      { key: "username", label: "Username", after: "giulia", monospace: true },
      { key: "display_name", label: "Display name", after: "Giulia Rossi" },
      { key: "roles", label: "Roles", after: "Operator, auditor" },
      { key: "status", label: "Status", after: "Active" },
    ]);
  });
});
