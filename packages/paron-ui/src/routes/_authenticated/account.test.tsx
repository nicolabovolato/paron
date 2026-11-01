import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import type {
  ApiError,
  CredentialsResponse,
  SignInResponse,
  StartedSignInResponse,
  UpdateCredentialsRequest,
  UpdateCredentialsResponse,
} from "@/generated/api/types.gen";
import { renderAppAt } from "@/test/app";
import { PASSWORD_ONLY } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/account", () => {
  test("authenticator app", async () => {
    const user = userEvent.setup();
    let confirmRequest: unknown;
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json(PASSWORD_ONLY),
      ),
      http.post<never, UpdateCredentialsRequest>(
        "/api/account/credentials",
        async ({ request }) => {
          const action = await request.json();
          if (action.action === "start_totp_registration") {
            return HttpResponse.json({
              result: {
                kind: "totp_registration_started",
                secret: "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
                key_uri:
                  "otpauth://totp/Paron%3Aadmin?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Paron",
              },
            } satisfies UpdateCredentialsResponse);
          }
          confirmRequest = action;
          return HttpResponse.json({
            result: { kind: "done" },
            recovery_codes: ["abcdefghjk", "mnpqrstvwx"],
          } satisfies UpdateCredentialsResponse);
        },
      ),
    );
    renderAppAt("/account");

    await user.click(await screen.findByRole("button", { name: "Set up" }));
    const setup = await screen.findByRole("dialog", {
      name: "Set up an authenticator app",
    });
    expect(
      within(setup).getByRole("img", {
        name: "QR code for the authenticator app",
      }),
    ).toBeInTheDocument();
    await user.type(
      within(setup).getByLabelText("Code from your authenticator app"),
      "123456",
    );
    await user.click(within(setup).getByRole("button", { name: "Turn on" }));

    const codes = await screen.findByRole("dialog", {
      name: "Your recovery codes",
    });
    expect(
      within(codes).getByText("abcde-fghjk mnpqr-stvwx", {
        normalizer: (text) => text.replace(/\s+/g, " "),
      }),
    ).toBeInTheDocument();
    expect(confirmRequest).toEqual({
      action: "finish_totp_registration",
      code: "123456",
    });
  });

  test("step-up", async () => {
    const user = userEvent.setup();
    let stepUpStarted = false;
    let signInRequest: unknown;
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json(PASSWORD_ONLY),
      ),
      http.post("/api/auth/sign-in", ({ request }) => {
        stepUpStarted =
          new URL(request.url).searchParams.get("step_up") === "true";
        return HttpResponse.json({
          accepted: [{ kind: "password" }],
        } satisfies StartedSignInResponse);
      }),
      http.post("/api/auth/sign-in/verify", async ({ request }) => {
        signInRequest = await request.json();
        return HttpResponse.json({ accepted: null } satisfies SignInResponse);
      }),
      http.post("/api/account/credentials", () =>
        signInRequest
          ? HttpResponse.json({
              result: {
                kind: "totp_registration_started",
                secret: "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
                key_uri:
                  "otpauth://totp/Paron%3Aadmin?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Paron",
              },
            } satisfies UpdateCredentialsResponse)
          : HttpResponse.json(
              {
                status: 403,
                code: "insufficient_user_authentication",
                detail: "this action needs a recent authentication",
              } satisfies ApiError,
              { status: 403 },
            ),
      ),
    );
    renderAppAt("/account");

    await user.click(await screen.findByRole("button", { name: "Set up" }));
    const reauthentication = await screen.findByRole("dialog", {
      name: "Confirm it is you",
    });
    await user.type(
      within(reauthentication).getByLabelText("Password"),
      "correct horse battery staple",
    );
    await user.click(
      within(reauthentication).getByRole("button", { name: "Sign in" }),
    );

    expect(
      await screen.findByRole("img", {
        name: "QR code for the authenticator app",
      }),
    ).toBeInTheDocument();
    expect(stepUpStarted).toBe(true);
    expect(signInRequest).toEqual({
      kind: "password",
      username: "admin",
      password: "correct horse battery staple",
    });
  });

  test("passkey removed", async () => {
    const user = userEvent.setup();
    let removeRequest: unknown;
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json({
          passkeys: [
            {
              id: "018f0000-0000-7000-8000-00000000aaaa",
              name: "Work laptop",
              created_at: "2026-01-01T00:00:00Z",
              last_used_at: null,
              backed_up: true,
            },
          ],
          recovery_codes: {
            generated_at: "2026-01-01T00:00:00Z",
            remaining: 2,
          },
        } satisfies CredentialsResponse),
      ),
      http.post("/api/account/credentials", async ({ request }) => {
        removeRequest = await request.json();
        return HttpResponse.json({
          result: { kind: "done" },
        } satisfies UpdateCredentialsResponse);
      }),
    );
    renderAppAt("/account");

    expect(await screen.findByText("2 left")).toBeInTheDocument();
    expect(screen.getByText("Synced")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Remove Work laptop" }),
    );
    const confirmation = await screen.findByRole("alertdialog", {
      name: "Remove the passkey Work laptop?",
    });
    await user.click(
      within(confirmation).getByRole("button", { name: "Remove" }),
    );

    await expect
      .poll(() => removeRequest)
      .toEqual({
        action: "remove_passkey",
        id: "018f0000-0000-7000-8000-00000000aaaa",
      });
  });

  test("passkey renamed", async () => {
    const user = userEvent.setup();
    let renameRequest: unknown;
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json({
          passkeys: [
            {
              id: "018f0000-0000-7000-8000-00000000aaaa",
              name: "Passkey",
              created_at: "2026-01-01T00:00:00Z",
              last_used_at: null,
              backed_up: true,
            },
          ],
        } satisfies CredentialsResponse),
      ),
      http.post("/api/account/credentials", async ({ request }) => {
        renameRequest = await request.json();
        return HttpResponse.json({
          result: { kind: "done" },
        } satisfies UpdateCredentialsResponse);
      }),
    );
    renderAppAt("/account");

    await user.click(
      await screen.findByRole("button", { name: "Rename Passkey" }),
    );
    const name = await screen.findByLabelText("Name of the passkey");
    await user.clear(name);
    await user.type(name, "Work laptop");
    await user.click(screen.getByRole("button", { name: "Rename Passkey" }));

    await expect
      .poll(() => renameRequest)
      .toEqual({
        action: "rename_passkey",
        id: "018f0000-0000-7000-8000-00000000aaaa",
        name: "Work laptop",
      });
  });

  test("password changed", async () => {
    const user = userEvent.setup();
    let setRequest: unknown;
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json(PASSWORD_ONLY),
      ),
      http.post("/api/account/credentials", async ({ request }) => {
        setRequest = await request.json();
        return HttpResponse.json({
          result: { kind: "done" },
        } satisfies UpdateCredentialsResponse);
      }),
    );
    renderAppAt("/account");

    await user.click(
      await screen.findByRole("button", { name: "Change password" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Change password",
    });
    await user.type(
      within(dialog).getByLabelText("New password"),
      "a long new password",
    );
    await user.type(
      within(dialog).getByLabelText("Confirm new password"),
      "a long new password",
    );
    await user.click(
      within(dialog).getByRole("button", { name: "Change password" }),
    );

    await expect
      .poll(() => setRequest)
      .toEqual({ action: "set_password", new_password: "a long new password" });
  });

  test("passkeys unavailable", async () => {
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json(PASSWORD_ONLY),
      ),
    );
    renderAppAt("/account");

    expect(
      await screen.findByRole("button", { name: "Add passkey" }),
    ).toHaveAttribute("aria-disabled", "true");
  });

  test("recovery codes used up", async () => {
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json({
          totp: {
            created_at: "2026-01-01T00:00:00Z",
            last_used_at: "2026-01-02T00:00:00Z",
          },
          passkeys: [],
          recovery_codes: {
            generated_at: "2026-01-01T00:00:00Z",
            remaining: 0,
          },
        } satisfies CredentialsResponse),
      ),
    );
    renderAppAt("/account");

    expect(await screen.findByText("0 left")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "New codes" }),
    ).toBeInTheDocument();
  });

  test("listed actions", async () => {
    server.use(
      http.get("/api/account/credentials", () =>
        HttpResponse.json({
          totp: {
            created_at: "2026-01-01T00:00:00Z",
            last_used_at: "2026-01-02T00:00:00Z",
          },
          passkeys: [],
        } satisfies CredentialsResponse),
      ),
    );
    renderAppAt("/account");

    expect(
      await screen.findByRole("button", { name: "Remove authenticator app" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Set up" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Change password" }),
    ).not.toBeInTheDocument();
  });
});
