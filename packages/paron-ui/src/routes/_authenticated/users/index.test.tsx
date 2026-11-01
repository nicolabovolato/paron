import { screen, waitFor, within } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { renderAppAt } from "@/test/app";
import {
  administratorWithout,
  EMPTY_PAGE,
  errorResponse,
  OPERATOR,
} from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

async function openAddUserDialog(user: UserEvent) {
  renderAppAt("/users");
  await user.click(await screen.findByRole("button", { name: "Add user" }));
  await screen.findByRole("dialog", { name: "Add user" });
}

async function fillValidUser(user: UserEvent) {
  await user.type(screen.getByLabelText("Username"), "giulia.r");
  await user.type(screen.getByLabelText("Display name"), "Giulia Rossi");
}

describe("/users", () => {
  test("200", async () => {
    server.use(
      http.get("/api/users", () =>
        HttpResponse.json({ items: [OPERATOR], total: 1 }),
      ),
    );

    renderAppAt("/users");

    const operatorRow = await screen.findByRole("row", {
      name: /Giulia Rossi/,
    });
    expect(
      within(operatorRow).getByRole("link", { name: /Giulia Rossi/ }),
    ).toHaveAttribute("href", `/users/${OPERATOR.id}`);
    expect(within(operatorRow).getByText("Operator")).toBeInTheDocument();
  });

  test("search", async () => {
    const user = userEvent.setup();
    server.use(
      http.get("/api/users", ({ request }) =>
        new URL(request.url).searchParams.get("search") === "nobody"
          ? HttpResponse.json(EMPTY_PAGE)
          : HttpResponse.json({ items: [OPERATOR], total: 1 }),
      ),
    );
    renderAppAt("/users");

    await user.type(
      await screen.findByRole("searchbox", { name: "Search users..." }),
      "nobody",
    );
    expect(
      await screen.findByText('No users found for "nobody"'),
    ).toBeInTheDocument();

    await user.click(screen.getByText("Clear search"));

    expect(
      await screen.findByRole("row", { name: /Giulia Rossi/ }),
    ).toBeInTheDocument();
    expect(screen.getByRole("searchbox")).toHaveValue("");
  });

  test("search too long", async () => {
    const user = userEvent.setup();
    const searches: (string | null)[] = [];
    server.use(
      http.get("/api/users", ({ request }) => {
        searches.push(new URL(request.url).searchParams.get("search"));
        return HttpResponse.json({ items: [OPERATOR], total: 1 });
      }),
    );
    renderAppAt("/users");

    await user.click(
      await screen.findByRole("searchbox", { name: "Search users..." }),
    );
    await user.paste("a".repeat(513));

    const searchbox = screen.getByRole("searchbox");
    await waitFor(() => expect(searchbox).toBeInvalid());
    expect(searchbox).toHaveAccessibleDescription(
      "Too big: expected string to have <=512 characters",
    );
    expect(searches).toEqual([null]);
  });

  test("no user:write", async () => {
    server.use(
      http.get("/api/auth/session", () =>
        HttpResponse.json(administratorWithout("user:write")),
      ),
      http.get("/api/users", () => HttpResponse.json(EMPTY_PAGE)),
    );

    renderAppAt("/users");

    expect(await screen.findByText("No users yet")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Add user" }),
    ).not.toBeInTheDocument();
  });

  describe("<AddUserDialog />", () => {
    test("required fields", async () => {
      const user = userEvent.setup();
      server.use(http.get("/api/users", () => HttpResponse.json(EMPTY_PAGE)));
      await openAddUserDialog(user);
      const usernameField = screen.getByLabelText("Username");

      await user.click(screen.getByRole("button", { name: "Create user" }));

      await waitFor(() =>
        expect(usernameField).toHaveAccessibleDescription(
          "Too small: expected string to have >=3 characters Letters, digits, and single '.', '-' or '_' between them. Stored in lower case.",
        ),
      );
      expect(screen.getByLabelText("Display name")).toHaveAccessibleDescription(
        "Too small: expected string to have >=1 characters",
      );
    });

    test("201", async () => {
      const user = userEvent.setup();
      let newUserRequest: unknown;
      server.use(
        http.get("/api/users", () => HttpResponse.json(EMPTY_PAGE)),
        http.post("/api/users", async ({ request }) => {
          newUserRequest = await request.json();
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
      await openAddUserDialog(user);
      await fillValidUser(user);

      await user.click(screen.getByRole("button", { name: "Create user" }));

      const linkDialog = await screen.findByRole("dialog", {
        name: "Recovery link for giulia.r",
      });
      expect(
        within(linkDialog).getByText(
          "https://paron.example.com/recover-account#token=a-link-token",
        ),
      ).toBeInTheDocument();
      expect(newUserRequest).toEqual({
        username: "giulia.r",
        display_name: "Giulia Rossi",
        roles: [],
      });
    });

    test("link 500", async () => {
      const user = userEvent.setup();
      server.use(
        http.get("/api/users", () => HttpResponse.json(EMPTY_PAGE)),
        http.post("/api/users", () =>
          HttpResponse.json(OPERATOR, { status: 201 }),
        ),
        http.post(`/api/users/${OPERATOR.id}/recovery`, () =>
          errorResponse({ status: 500 }),
        ),
      );
      await openAddUserDialog(user);
      await fillValidUser(user);

      await user.click(screen.getByRole("button", { name: "Create user" }));

      expect(
        await screen.findByText("Couldn't create the recovery link"),
      ).toBeInTheDocument();
      await waitFor(() =>
        expect(
          screen.queryByRole("dialog", { name: "Add user" }),
        ).not.toBeInTheDocument(),
      );
    });

    test("409", async () => {
      const user = userEvent.setup();
      server.use(
        http.get("/api/users", () => HttpResponse.json(EMPTY_PAGE)),
        http.post("/api/users", () =>
          errorResponse({
            status: 409,
            detail: "entity already exists",
          }),
        ),
      );
      await openAddUserDialog(user);
      await fillValidUser(user);

      await user.click(screen.getByRole("button", { name: "Create user" }));

      expect(
        await screen.findByText("Couldn't create the user"),
      ).toBeInTheDocument();
      expect(screen.getByText("Entity already exists")).toBeInTheDocument();
      expect(screen.getByLabelText("Username")).toBeValid();
      expect(
        screen.getByRole("dialog", { name: "Add user" }),
      ).toBeInTheDocument();
    });

    test("422", async () => {
      const user = userEvent.setup();
      server.use(
        http.get("/api/users", () => HttpResponse.json(EMPTY_PAGE)),
        http.post("/api/users", () =>
          errorResponse({
            status: 422,
            detail: "validation failed",
            validation_errors: { username: ["reserved name"] },
          }),
        ),
      );
      await openAddUserDialog(user);
      await fillValidUser(user);
      const usernameField = screen.getByLabelText("Username");

      await user.click(screen.getByRole("button", { name: "Create user" }));

      await waitFor(() =>
        expect(usernameField).toHaveAccessibleDescription("reserved name"),
      );
    });
  });
});
