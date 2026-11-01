import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { renderAppAt } from "@/test/app";
import { MACHINE } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/machines/$machineId/edit", () => {
  test("changed fields", async () => {
    const user = userEvent.setup();
    let machinePatch: unknown;
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
      http.patch(`/api/machines/${MACHINE.id}`, async ({ request }) => {
        machinePatch = await request.json();
        return HttpResponse.json(MACHINE);
      }),
    );
    renderAppAt(`/machines/${MACHINE.id}/edit`);
    const userField = await screen.findByLabelText("User");

    await user.clear(userField);
    await user.type(userField, "root");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(machinePatch).toEqual({
        protocols: { ssh: { user: "root" } },
      }),
    );
  });

  test("SSH added", async () => {
    const user = userEvent.setup();
    const machineWithoutSsh = {
      ...MACHINE,
      protocols: { ssh: null },
      capabilities: { connection_test: [], terminal: [] },
    };
    let machinePatch: unknown;
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () =>
        HttpResponse.json(machineWithoutSsh),
      ),
      http.patch(`/api/machines/${MACHINE.id}`, async ({ request }) => {
        machinePatch = await request.json();
        return HttpResponse.json(MACHINE);
      }),
    );
    renderAppAt(`/machines/${MACHINE.id}/edit`);

    await user.type(await screen.findByLabelText("User"), "deploy");
    await user.type(
      screen.getByLabelText("Host key"),
      MACHINE.protocols.ssh.authentication.host_key,
    );
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(machinePatch).toEqual({
        protocols: {
          ssh: {
            user: "deploy",
            port: 22,
            connect_timeout_seconds: 10,
            authentication: {
              host_key: MACHINE.protocols.ssh.authentication.host_key,
              method: {
                kind: "keypair",
                source: { kind: "managed", algorithm: "ed25519" },
              },
            },
          },
        },
      }),
    );
  });

  test("host key", async () => {
    const user = userEvent.setup();
    const newHostKey = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOtherHostKey";
    let machinePatch: unknown;
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
      http.patch(`/api/machines/${MACHINE.id}`, async ({ request }) => {
        machinePatch = await request.json();
        return HttpResponse.json(MACHINE);
      }),
    );
    renderAppAt(`/machines/${MACHINE.id}/edit`);
    const hostKeyField = await screen.findByLabelText("Host key");

    await user.clear(hostKeyField);
    await user.type(hostKeyField, newHostKey);
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(machinePatch).toEqual({
        protocols: {
          ssh: { authentication: { host_key: newHostKey } },
        },
      }),
    );
  });

  test("key kept", async () => {
    const user = userEvent.setup();
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
    );
    renderAppAt(`/machines/${MACHINE.id}/edit`);
    await user.click(await screen.findByRole("button", { name: "Replace" }));
    expect(
      await screen.findByText("Saving replaces the current key"),
    ).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Keep the current one" }),
    );

    expect(
      await screen.findByText("Paron keeps using it until you replace it."),
    ).toBeInTheDocument();
  });
});
