import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, test } from "vitest";

import type {
  AuditEntryResponse,
  CredentialsResponse,
  UserResponse,
} from "@/generated/api/types.gen";
import { renderAppAt } from "@/test/app";
import { ADMINISTRATOR, MACHINE, OPERATOR } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/", () => {
  afterEach(() => localStorage.clear());

  test("setup", async () => {
    renderAppAt("/");

    expect(
      await screen.findByRole("heading", { name: "Set up Paron" }),
    ).toBeInTheDocument();
    expect(screen.getByText("0 of 4 done")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "New machine" })).toHaveAttribute(
      "href",
      "/machines/new",
    );
    expect(
      screen.getByRole("button", { name: "Open terminal" }),
    ).toHaveAttribute("aria-disabled", "true");
  });

  test("done", async () => {
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json({
          passkeys: [
            {
              id: "018f0000-0000-7000-8000-0000000000c1",
              name: "Laptop",
              created_at: "2026-01-01T00:00:00Z",
              backed_up: true,
            },
          ],
        } satisfies CredentialsResponse),
      ),
      http.get("/api/machines", () =>
        HttpResponse.json({ items: [MACHINE], total: 1 }),
      ),
      http.get("/api/audit", () =>
        HttpResponse.json({
          items: [
            {
              id: "018f0000-0000-7000-8000-0000000000a2",
              created_at: "2026-10-08T12:00:00.000Z",
              actor: { type: "user", id: ADMINISTRATOR.id, name: "admin" },
              target: { type: "machine", id: MACHINE.id, name: MACHINE.name },
              event: {
                type: "machine.terminal_opened",
                payload: {
                  session_id: "018f0000-0000-7000-8000-0000000000d1",
                  protocol: "ssh",
                },
              },
            } satisfies AuditEntryResponse,
          ],
        }),
      ),
      http.get("/api/users", () =>
        HttpResponse.json({ items: [ADMINISTRATOR, OPERATOR], total: 2 }),
      ),
    );

    renderAppAt("/");

    expect(
      await screen.findByRole("heading", { name: "Paron is set up" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Show setup" }),
    ).not.toBeInTheDocument();
    expect(localStorage.getItem(`paron-setup:${ADMINISTRATOR.id}`)).toBe(
      "done",
    );
  });

  test("skip", async () => {
    const user = userEvent.setup();
    renderAppAt("/");

    await user.click(await screen.findByRole("button", { name: "Skip setup" }));
    await user.click(await screen.findByRole("button", { name: "Show setup" }));

    expect(
      await screen.findByRole("heading", { name: "Set up Paron" }),
    ).toBeInTheDocument();
  });

  test("add user", async () => {
    const user = userEvent.setup();
    let users: UserResponse[] = [ADMINISTRATOR];
    server.use(
      http.get("/api/users", () =>
        HttpResponse.json({ items: users, total: users.length }),
      ),
      http.post("/api/users", () => {
        users = [ADMINISTRATOR, OPERATOR];
        return HttpResponse.json(OPERATOR, { status: 201 });
      }),
      http.post(`/api/users/${OPERATOR.id}/recovery`, () =>
        HttpResponse.json({
          token: "a-link-token",
          public_origin: "https://paron.example.com",
          expires_at: "2099-01-01T00:00:00Z",
        }),
      ),
    );
    renderAppAt("/");

    await user.click(await screen.findByRole("button", { name: "Add user" }));
    await user.type(screen.getByLabelText("Username"), "giulia.r");
    await user.type(screen.getByLabelText("Display name"), "Giulia Rossi");
    await user.click(screen.getByRole("button", { name: "Create user" }));

    expect(await screen.findByText("1 of 4 done")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Add user" }),
    ).not.toBeInTheDocument();
  });

  test("operator", async () => {
    server.use(
      http.get("/api/auth/session", () => HttpResponse.json(OPERATOR)),
    );

    renderAppAt("/");

    expect(await screen.findByText("0 of 3 done")).toBeInTheDocument();
    expect(screen.queryByText("Add a teammate")).not.toBeInTheDocument();
  });

  test("stored", async () => {
    let machineRequestCount = 0;
    server.use(
      http.get("/api/machines", () => {
        machineRequestCount++;
        return HttpResponse.json({ items: [], total: 0 });
      }),
    );
    localStorage.setItem(`paron-setup:${ADMINISTRATOR.id}`, "done");

    renderAppAt("/");

    expect(
      await screen.findByRole("heading", { name: "Paron is set up" }),
    ).toBeInTheDocument();
    expect(machineRequestCount).toBe(0);
  });
});
