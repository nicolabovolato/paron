import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import type { CredentialsResponse } from "@/generated/api/types.gen";
import { renderAppAt } from "@/test/app";
import { ADMINISTRATOR, OPERATOR } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/users/$userId", () => {
  test("recovery link", async () => {
    const user = userEvent.setup();
    let recoveryStarted = false;
    server.use(
      http.get(`/api/users/${OPERATOR.id}`, () => HttpResponse.json(OPERATOR)),
      http.get(`/api/users/${OPERATOR.id}/credentials`, () =>
        HttpResponse.json({
          password: { updated_at: "2026-01-01T00:00:00Z" },
          passkeys: [
            {
              id: "018f0000-0000-7000-8000-00000000aaaa",
              name: "Work laptop",
              created_at: "2026-01-01T00:00:00Z",
              backed_up: true,
            },
            {
              id: "018f0000-0000-7000-8000-00000000bbbb",
              name: "Phone",
              created_at: "2026-01-02T00:00:00Z",
              backed_up: false,
            },
          ],
        } satisfies CredentialsResponse),
      ),
      http.post(`/api/users/${OPERATOR.id}/recovery`, () => {
        recoveryStarted = true;
        return HttpResponse.json({
          token: "a-link-token",
          public_origin: "https://paron.example.com",
          expires_at: "2099-01-01T00:00:00Z",
        });
      }),
    );
    renderAppAt(`/users/${OPERATOR.id}`);

    expect(await screen.findByText("2 passkeys")).toBeInTheDocument();
    await user.click(
      await screen.findByRole("button", { name: "Create recovery link" }),
    );
    const confirmation = await screen.findByRole("alertdialog", {
      name: "Create a recovery link for giulia.r?",
    });
    expect(recoveryStarted).toBe(false);
    await user.click(
      within(confirmation).getByRole("button", {
        name: "Remove and create link",
      }),
    );

    const linkDialog = await screen.findByRole("dialog", {
      name: "Recovery link for giulia.r",
    });
    expect(
      within(linkDialog).getByText(
        "https://paron.example.com/recover-account#token=a-link-token",
      ),
    ).toBeInTheDocument();
    expect(recoveryStarted).toBe(true);
  });

  test("own account", async () => {
    server.use(
      http.get(`/api/users/${ADMINISTRATOR.id}`, () =>
        HttpResponse.json(ADMINISTRATOR),
      ),
    );
    renderAppAt(`/users/${ADMINISTRATOR.id}`);

    expect(await screen.findByText("Profile")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Create recovery link" }),
    ).not.toBeInTheDocument();
  });
});
