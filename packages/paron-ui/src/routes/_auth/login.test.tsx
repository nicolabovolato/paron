import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import type {
  SignInCredentialRequest,
  SignInResponse,
} from "@/generated/api/types.gen";
import { renderAppAt } from "@/test/app";
import {
  ADMINISTRATOR,
  errorResponse,
  noSessionResponse,
} from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

const SIGNED_IN = {
  accepted: null,
} satisfies SignInResponse;

const PASSWORD_ACCEPTED = {
  accepted: [{ kind: "totp" }, { kind: "recovery_code" }],
} satisfies SignInResponse;

describe("/login", () => {
  test("200", async () => {
    const user = userEvent.setup();
    let signedIn = false;
    server.use(
      http.get("/api/auth/session", () =>
        signedIn ? HttpResponse.json(ADMINISTRATOR) : noSessionResponse(),
      ),
      http.post("/api/auth/sign-in/verify", () => {
        signedIn = true;
        return HttpResponse.json(SIGNED_IN);
      }),
    );
    renderAppAt("/");

    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(
      await screen.findByRole("heading", { name: "Overview" }),
    ).toBeInTheDocument();
  });

  test("Enter", async () => {
    const user = userEvent.setup();
    let signedIn = false;
    server.use(
      http.get("/api/auth/session", () =>
        signedIn ? HttpResponse.json(ADMINISTRATOR) : noSessionResponse(),
      ),
      http.post("/api/auth/sign-in/verify", () => {
        signedIn = true;
        return HttpResponse.json(SIGNED_IN);
      }),
    );
    renderAppAt("/");

    // Tab validates the form while the password is empty. Enter does not blur the password
    // field, so that error must clear while the user types.
    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.tab();
    await user.keyboard("correct horse battery{Enter}");

    expect(
      await screen.findByRole("heading", { name: "Overview" }),
    ).toBeInTheDocument();
  });

  test("400", async () => {
    const user = userEvent.setup();
    server.use(
      http.get("/api/auth/session", noSessionResponse),
      http.post("/api/auth/sign-in/verify", () =>
        errorResponse({
          status: 400,
          detail: "the credentials are not correct",
        }),
      ),
    );
    renderAppAt("/");

    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.type(screen.getByLabelText("Password"), "not the right one");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(
      await screen.findByText("The credentials are not correct"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Username")).toBeInTheDocument();
  });

  test("401", async () => {
    const user = userEvent.setup();
    let starts = 0;
    server.use(
      http.get("/api/auth/session", noSessionResponse),
      http.post("/api/auth/sign-in", () => {
        starts++;
        return HttpResponse.json({ accepted: [{ kind: "password" }] });
      }),
      http.post("/api/auth/sign-in/verify", () =>
        errorResponse({ status: 401, detail: "sign-in ended" }),
      ),
    );
    renderAppAt("/login");

    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));

    expect(await screen.findByText("Sign-in ended")).toBeInTheDocument();
    expect(await screen.findByLabelText("Username")).toBeInTheDocument();
    expect(starts).toBe(2);
  });

  test("second factor", async () => {
    const user = userEvent.setup();
    let signedIn = false;
    let secondFactorRequest: unknown;
    server.use(
      http.get("/api/auth/session", () =>
        signedIn ? HttpResponse.json(ADMINISTRATOR) : noSessionResponse(),
      ),
      http.post<never, SignInCredentialRequest>(
        "/api/auth/sign-in/verify",
        async ({ request }) => {
          const credential = await request.json();
          if (credential.kind === "password")
            return HttpResponse.json(PASSWORD_ACCEPTED);
          secondFactorRequest = credential;
          signedIn = true;
          return HttpResponse.json(SIGNED_IN);
        },
      ),
    );
    renderAppAt("/");

    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await user.type(
      await screen.findByLabelText("Code from your authenticator app"),
      "123456",
    );
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      await screen.findByRole("heading", { name: "Overview" }),
    ).toBeInTheDocument();
    expect(secondFactorRequest).toEqual({
      kind: "totp",
      code: "123456",
    });
  });

  test("recovery code", async () => {
    const user = userEvent.setup();
    let secondFactorRequest: unknown;
    server.use(
      http.get("/api/auth/session", noSessionResponse),
      http.post<never, SignInCredentialRequest>(
        "/api/auth/sign-in/verify",
        async ({ request }) => {
          const credential = await request.json();
          if (credential.kind === "password")
            return HttpResponse.json(PASSWORD_ACCEPTED);
          secondFactorRequest = credential;
          return errorResponse({
            status: 400,
            detail: "the credentials are not correct",
          });
        },
      ),
    );
    renderAppAt("/");

    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.type(screen.getByLabelText("Password"), "correct horse battery");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await user.click(
      await screen.findByRole("button", { name: "Use a recovery code" }),
    );
    await user.type(
      screen.getByLabelText("One of your recovery codes"),
      "abcde-fghjk",
    );
    await user.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      await screen.findByText("The credentials are not correct"),
    ).toBeInTheDocument();
    expect(secondFactorRequest).toEqual({
      kind: "recovery_code",
      code: "abcdefghjk",
    });
  });

  test("invalid redirect", async () => {
    server.use(http.get("/api/auth/session", noSessionResponse));

    const router = renderAppAt("/login?redirect=5");

    expect(await screen.findByLabelText("Username")).toBeInTheDocument();
    expect(router.state.location.search).not.toHaveProperty("redirect", 5);
  });
});
