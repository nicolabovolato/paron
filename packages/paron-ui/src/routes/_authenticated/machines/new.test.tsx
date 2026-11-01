import { screen, waitFor } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { renderAppAt } from "@/test/app";
import { administratorWithout, errorResponse, MACHINE } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

const HOST_KEY = MACHINE.protocols.ssh.authentication.host_key;

async function fillValidMachine(user: UserEvent) {
  await user.type(await screen.findByLabelText("Name"), "forno-01");
  await user.type(screen.getByLabelText("Host"), "10.0.0.5");
  await user.type(screen.getByLabelText("User"), "deploy");
  await user.type(screen.getByLabelText("Host key"), HOST_KEY);
}

describe("/machines/new", () => {
  test("required fields", async () => {
    const user = userEvent.setup();
    renderAppAt("/machines/new");
    const nameField = await screen.findByLabelText("Name");

    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() =>
      expect(nameField).toHaveAccessibleDescription(
        "Too small: expected string to have >=1 characters",
      ),
    );
    expect(screen.getByLabelText("Host")).toHaveAccessibleDescription(
      "A hostname or IP address, without a scheme or a path.",
    );
    expect(screen.getByLabelText("User")).toHaveAccessibleDescription(
      "Too small: expected string to have >=1 characters",
    );
    expect(screen.getByLabelText("Host key")).toHaveAccessibleDescription(
      "An SSH public key, exactly as `ssh-keyscan` prints it: `ssh-ed25519 AAAA...`.",
    );
  });

  test("empty port", async () => {
    const user = userEvent.setup();
    renderAppAt("/machines/new");
    await fillValidMachine(user);
    const portField = screen.getByLabelText("Port");

    await user.clear(portField);
    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() =>
      expect(portField).toHaveAccessibleDescription(
        "Invalid input: expected number, received NaN",
      ),
    );
  });

  test("tunneled", async () => {
    const user = userEvent.setup();
    renderAppAt("/machines/new");
    await screen.findByLabelText("Host");

    await user.click(screen.getByRole("radio", { name: /Tunneled/ }));

    expect(screen.queryByLabelText("Host")).not.toBeInTheDocument();
  });

  test("imported key", async () => {
    const user = userEvent.setup();
    renderAppAt("/machines/new");
    await fillValidMachine(user);

    await user.click(screen.getByRole("radio", { name: /Import a key/ }));
    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() =>
      expect(screen.getByLabelText("Private key")).toHaveAccessibleDescription(
        "An OpenSSH private key, exactly as `ssh-keygen` wrote the file. Must not be passphrase-protected.",
      ),
    );
  });

  test("422", async () => {
    const user = userEvent.setup();
    server.use(
      http.post("/api/machines", () =>
        errorResponse({
          status: 422,
          detail: "validation failed",
          validation_errors: { "protocols.ssh.user": ["no such user"] },
        }),
      ),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);

    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() =>
      expect(screen.getByLabelText("User")).toHaveAccessibleDescription(
        "no such user",
      ),
    );
  });

  test("422 then corrected", async () => {
    const user = userEvent.setup();
    let attempts = 0;
    server.use(
      http.post("/api/machines", () => {
        attempts += 1;
        return attempts === 1
          ? errorResponse({
              status: 422,
              detail: "validation failed",
              validation_errors: { "protocols.ssh.user": ["no such user"] },
            })
          : HttpResponse.json(MACHINE, { status: 201 });
      }),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);
    const userField = screen.getByLabelText("User");
    await user.click(screen.getByRole("button", { name: "Create machine" }));
    await waitFor(() =>
      expect(userField).toHaveAccessibleDescription("no such user"),
    );

    await user.type(userField, "x");
    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() => expect(attempts).toBe(2));
  });

  test("409", async () => {
    const user = userEvent.setup();
    server.use(
      http.post("/api/machines", () =>
        errorResponse({
          status: 409,
          detail: "entity already exists",
        }),
      ),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);

    await user.click(screen.getByRole("button", { name: "Create machine" }));

    expect(
      await screen.findByText("Couldn't save the machine"),
    ).toBeInTheDocument();
    expect(screen.getByText("Entity already exists")).toBeInTheDocument();
    expect(screen.getByLabelText("Name")).toBeValid();
  });

  test("201", async () => {
    const user = userEvent.setup();
    let newMachineRequest: unknown;
    server.use(
      http.post("/api/machines", async ({ request }) => {
        newMachineRequest = await request.json();
        return HttpResponse.json(MACHINE, { status: 201 });
      }),
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);

    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() =>
      expect(newMachineRequest).toEqual({
        name: "forno-01",
        connectivity: { kind: "direct", host: "10.0.0.5" },
        protocols: {
          ssh: {
            user: "deploy",
            port: 22,
            connect_timeout_seconds: 10,
            authentication: {
              host_key: HOST_KEY,
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

  test("Enter", async () => {
    const user = userEvent.setup();
    let newMachineRequest: unknown;
    server.use(
      http.post("/api/machines", async ({ request }) => {
        newMachineRequest = await request.json();
        return HttpResponse.json(MACHINE, { status: 201 });
      }),
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);

    await user.type(screen.getByLabelText("User"), "{Enter}");

    await waitFor(() =>
      expect(newMachineRequest).toMatchObject({ name: "forno-01" }),
    );
  });

  test("connection OK", async () => {
    const user = userEvent.setup();
    let testConnectionRequest: unknown;
    server.use(
      http.post("/api/machines/test-connection", async ({ request }) => {
        testConnectionRequest = await request.json();
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);
    await user.click(screen.getByRole("tab", { name: "Password" }));
    await user.type(
      screen.getByLabelText("Password", { selector: "input" }),
      "secret",
    );

    await user.click(screen.getByRole("button", { name: "Test connection" }));

    expect(await screen.findByText("Connection OK")).toBeInTheDocument();
    expect(testConnectionRequest).toMatchObject({
      name: "forno-01",
      connectivity: { kind: "direct", host: "10.0.0.5" },
    });
  });

  test("connection required fields", async () => {
    const user = userEvent.setup();
    let requested = false;
    server.use(
      http.post("/api/machines/test-connection", () => {
        requested = true;
        return new HttpResponse(null, { status: 204 });
      }),
      http.post("/api/machines", () => {
        requested = true;
        return HttpResponse.json(MACHINE, { status: 201 });
      }),
    );
    renderAppAt("/machines/new");
    const nameField = await screen.findByLabelText("Name");
    await user.click(screen.getByRole("tab", { name: "Password" }));

    await user.click(screen.getByRole("button", { name: "Test connection" }));

    await waitFor(() =>
      expect(nameField).toHaveAccessibleDescription(
        "Too small: expected string to have >=1 characters",
      ),
    );
    expect(screen.getByLabelText("User")).toHaveAccessibleDescription(
      "Too small: expected string to have >=1 characters",
    );
    expect(requested).toBe(false);
  });

  test("save during a connection test", async () => {
    const user = userEvent.setup();
    let machineCreated = false;
    server.use(
      http.post("/api/machines/test-connection", () => new Promise(() => {})),
      http.post("/api/machines", () => {
        machineCreated = true;
        return HttpResponse.json(MACHINE, { status: 201 });
      }),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);
    await user.click(screen.getByRole("tab", { name: "Password" }));
    await user.type(
      screen.getByLabelText("Password", { selector: "input" }),
      "secret",
    );
    await user.click(screen.getByRole("button", { name: "Test connection" }));

    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() => expect(machineCreated).toBe(true));
  });

  test("connection test during a save", async () => {
    const user = userEvent.setup();
    server.use(http.post("/api/machines", () => new Promise(() => {})));
    renderAppAt("/machines/new");
    await fillValidMachine(user);
    await user.click(screen.getByRole("tab", { name: "Password" }));
    await user.type(
      screen.getByLabelText("Password", { selector: "input" }),
      "secret",
    );

    await user.click(screen.getByRole("button", { name: "Create machine" }));

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Test connection" }),
      ).toHaveAttribute("aria-disabled", "true"),
    );
  });

  test("connection 422", async () => {
    const user = userEvent.setup();
    let machineCreated = false;
    server.use(
      http.post("/api/machines/test-connection", () =>
        errorResponse({
          status: 422,
          detail: "validation failed",
          validation_errors: {
            "protocols.ssh.authentication.host_key": ["not a valid SSH key"],
          },
        }),
      ),
      http.post("/api/machines", () => {
        machineCreated = true;
        return HttpResponse.json(MACHINE, { status: 201 });
      }),
    );
    renderAppAt("/machines/new");
    await fillValidMachine(user);
    const hostKeyField = screen.getByLabelText("Host key");
    await user.click(screen.getByRole("tab", { name: "Password" }));
    await user.type(
      screen.getByLabelText("Password", { selector: "input" }),
      "secret",
    );

    await user.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() =>
      expect(hostKeyField).toHaveAccessibleDescription("not a valid SSH key"),
    );

    await user.type(hostKeyField, "x");
    await user.tab();
    expect(hostKeyField).toBeValid();

    await user.click(screen.getByRole("button", { name: "Create machine" }));
    await waitFor(() => expect(machineCreated).toBe(true));
  });

  test("no machine:operate", async () => {
    server.use(
      http.get("/api/auth/session", () =>
        HttpResponse.json(administratorWithout("machine:operate")),
      ),
    );

    renderAppAt("/machines/new");

    expect(await screen.findByLabelText("Name")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Test connection" }),
    ).not.toBeInTheDocument();
  });
});
