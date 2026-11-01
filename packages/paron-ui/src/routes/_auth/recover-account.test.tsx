import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, test, vi } from "vitest";

import type { RecoveredAccountResponse } from "@/generated/api/types.gen";
import { renderAppAt } from "@/test/app";
import {
  ADMINISTRATOR,
  errorResponse,
  noSessionResponse,
} from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

afterEach(() => {
  vi.unstubAllGlobals();
  Reflect.deleteProperty(navigator, "credentials");
});

describe("/recover-account", () => {
  test("200", async () => {
    const user = userEvent.setup();
    let recoveryRequest: unknown;
    server.use(
      http.get("/api/auth/session", () => noSessionResponse()),
      http.post("/api/auth/recovery", async ({ request }) => {
        recoveryRequest = {
          authorization: request.headers.get("Authorization"),
          body: await request.json(),
        };
        return HttpResponse.json({
          kind: "done",
        } satisfies RecoveredAccountResponse);
      }),
    );
    renderAppAt("/recover-account#token=a-link-token");

    await user.type(
      await screen.findByLabelText("New password"),
      "correct horse battery",
    );
    await user.type(
      screen.getByLabelText("Confirm new password"),
      "correct horse battery",
    );
    await user.click(screen.getByRole("button", { name: "Set password" }));

    expect(
      await screen.findByText(
        "Your account is recovered. Sign in with your new password.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Welcome back" }),
    ).toBeInTheDocument();
    expect(recoveryRequest).toEqual({
      authorization: "Bearer a-link-token",
      body: { action: "set_password", password: "correct horse battery" },
    });
  });

  test("400", async () => {
    server.use(
      http.get("/api/auth/session", () => noSessionResponse()),
      http.get("/api/auth/recovery", () =>
        errorResponse({
          status: 400,
          detail: "the token is not correct",
        }),
      ),
    );
    renderAppAt("/recover-account#token=a-used-token");

    expect(
      await screen.findByRole("heading", { name: "This link does not work" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("New password")).not.toBeInTheDocument();
  });

  test("passwords differ", async () => {
    const user = userEvent.setup();
    server.use(http.get("/api/auth/session", () => noSessionResponse()));
    renderAppAt("/recover-account#token=a-link-token");

    await user.type(
      await screen.findByLabelText("New password"),
      "correct horse battery",
    );
    await user.type(
      screen.getByLabelText("Confirm new password"),
      "correct horse battery!",
    );
    await user.click(screen.getByRole("button", { name: "Set password" }));

    await waitFor(() =>
      expect(
        screen.getByLabelText("Confirm new password"),
      ).toHaveAccessibleDescription("Passwords do not match"),
    );
  });

  test("passkey", async () => {
    const user = userEvent.setup();
    const recoveryBodies: unknown[] = [];
    class FakePublicKeyCredential {
      static parseCreationOptionsFromJSON(options: unknown) {
        return options;
      }
      static parseRequestOptionsFromJSON(options: unknown) {
        return options;
      }
      toJSON() {
        return { id: "a-passkey" };
      }
    }
    vi.stubGlobal("PublicKeyCredential", FakePublicKeyCredential);
    Object.defineProperty(navigator, "credentials", {
      configurable: true,
      value: { create: () => Promise.resolve(new FakePublicKeyCredential()) },
    });
    server.use(
      http.get("/api/auth/session", () => noSessionResponse()),
      http.post<never, { action: string }>(
        "/api/auth/recovery",
        async ({ request }) => {
          const body = await request.json();
          recoveryBodies.push(body);
          return HttpResponse.json(
            (body.action === "start_passkey_registration"
              ? {
                  kind: "passkey_creation_options",
                  options: { challenge: "abc" },
                }
              : { kind: "done" }) satisfies RecoveredAccountResponse,
          );
        },
      ),
    );
    renderAppAt("/recover-account#token=a-link-token");

    await user.click(
      await screen.findByRole("button", { name: "Use a passkey instead" }),
    );

    expect(
      await screen.findByText(
        "Your account is recovered. Sign in with your new passkey.",
      ),
    ).toBeInTheDocument();
    expect(recoveryBodies).toEqual([
      { action: "start_passkey_registration" },
      {
        action: "finish_passkey_registration",
        response: { id: "a-passkey" },
      },
    ]);
  });

  test("signed in", async () => {
    const user = userEvent.setup();
    let signedOut = false;
    server.use(
      http.get("/api/auth/session", () =>
        signedOut ? noSessionResponse() : HttpResponse.json(ADMINISTRATOR),
      ),
      http.delete("/api/auth/session", () => {
        signedOut = true;
        return new HttpResponse(null, { status: 204 });
      }),
    );
    renderAppAt("/recover-account#token=a-link-token");

    expect(
      await screen.findByText(
        `You are signed in as ${ADMINISTRATOR.username}. Sign out to use this link.`,
      ),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("New password")).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Sign out and continue" }),
    );

    expect(await screen.findByLabelText("New password")).toBeInTheDocument();
    expect(signedOut).toBe(true);
  });

  test("no token", async () => {
    renderAppAt("/recover-account");

    expect(
      await screen.findByRole("heading", { name: "This link does not work" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("New password")).not.toBeInTheDocument();
  });
});
