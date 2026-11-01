import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { renderAppAt } from "@/test/app";
import { errorResponse, OPERATOR } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/users/$userId/edit", () => {
  test("changed fields", async () => {
    const user = userEvent.setup();
    let userPatch: unknown;
    server.use(
      http.get(`/api/users/${OPERATOR.id}`, () => HttpResponse.json(OPERATOR)),
      http.get("/api/users", () =>
        HttpResponse.json({ items: [OPERATOR], total: 1 }),
      ),
      http.patch(`/api/users/${OPERATOR.id}`, async ({ request }) => {
        userPatch = await request.json();
        return HttpResponse.json(OPERATOR);
      }),
    );
    renderAppAt(`/users/${OPERATOR.id}/edit`);
    const displayNameField = await screen.findByLabelText("Display name");

    await user.clear(displayNameField);
    await user.type(displayNameField, "Giulia R.");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(userPatch).toEqual({ display_name: "Giulia R." }),
    );
  });

  test("refetched", async () => {
    const user = userEvent.setup();
    let storedUser = OPERATOR;
    let userPatch: unknown;
    server.use(
      http.get(`/api/users/${OPERATOR.id}`, () =>
        HttpResponse.json(storedUser),
      ),
      http.patch(`/api/users/${OPERATOR.id}`, async ({ request }) => {
        userPatch = await request.json();
        return HttpResponse.json(storedUser);
      }),
    );
    const router = renderAppAt(`/users/${OPERATOR.id}/edit`);
    const usernameField = await screen.findByLabelText("Username");

    await user.clear(usernameField);
    await user.type(usernameField, "giulia.b");
    storedUser = { ...OPERATOR, display_name: "Giulia Bianchi" };
    await act(() => router.options.context.queryClient.invalidateQueries());
    await screen.findByRole("heading", { name: "Edit Giulia Bianchi" });
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(userPatch).toEqual({ username: "giulia.b" }));
  });

  test("409", async () => {
    const user = userEvent.setup();
    server.use(
      http.get(`/api/users/${OPERATOR.id}`, () => HttpResponse.json(OPERATOR)),
      http.patch(`/api/users/${OPERATOR.id}`, () =>
        errorResponse({
          status: 409,
          detail: "entity already exists",
        }),
      ),
    );
    renderAppAt(`/users/${OPERATOR.id}/edit`);
    const usernameField = await screen.findByLabelText("Username");

    await user.clear(usernameField);
    await user.type(usernameField, "admin");
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    expect(
      await screen.findByText("Couldn't save the user"),
    ).toBeInTheDocument();
    expect(screen.getByText("Entity already exists")).toBeInTheDocument();
    expect(usernameField).toBeValid();
  });

  test("empty display name", async () => {
    const user = userEvent.setup();
    server.use(
      http.get(`/api/users/${OPERATOR.id}`, () => HttpResponse.json(OPERATOR)),
    );
    renderAppAt(`/users/${OPERATOR.id}/edit`);
    const displayNameField = await screen.findByLabelText("Display name");

    await user.clear(displayNameField);
    await user.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(displayNameField).toHaveAccessibleDescription(
        "Too small: expected string to have >=1 characters",
      ),
    );
  });
});
