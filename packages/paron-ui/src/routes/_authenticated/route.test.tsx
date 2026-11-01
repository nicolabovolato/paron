import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { authQueries } from "@/lib/auth";
import { renderAppAt } from "@/test/app";
import { ADMINISTRATOR, MACHINE, noSessionResponse } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/_authenticated", () => {
  test("200", async () => {
    renderAppAt("/");

    expect(
      await screen.findByRole("heading", { name: "Overview" }),
    ).toBeInTheDocument();
  });

  test("title", async () => {
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
    );

    renderAppAt(`/machines/${MACHINE.id}/edit`);

    await waitFor(() =>
      expect(document.title).toBe("Edit - forno-01 - Machines - Paron"),
    );
  });

  test("401", async () => {
    server.use(http.get("/api/auth/session", noSessionResponse));

    renderAppAt("/");

    expect(await screen.findByLabelText("Username")).toBeInTheDocument();
    expect(screen.queryByText(/session has expired/)).not.toBeInTheDocument();
  });

  test("401 redirect", async () => {
    server.use(http.get("/api/auth/session", noSessionResponse));

    const router = renderAppAt("/users");

    await screen.findByLabelText("Username");
    await waitFor(() => {
      expect(router.state.location.pathname).toBe("/login");
      expect(router.state.location.search).toMatchObject({
        redirect: "/users",
      });
    });
  });

  test("sign out", async () => {
    const user = userEvent.setup();
    let signedIn = true;
    let meRequestCount = 0;
    server.use(
      http.get("/api/auth/session", () => {
        meRequestCount++;
        return signedIn
          ? HttpResponse.json(ADMINISTRATOR)
          : noSessionResponse();
      }),
      http.delete("/api/auth/session", () => {
        signedIn = false;
        return new Response(null, { status: 204 });
      }),
    );
    const router = renderAppAt("/");
    await user.click(await screen.findByRole("button", { name: /Admin/ }));

    await user.click(await screen.findByRole("menuitem", { name: "Sign out" }));

    expect(await screen.findByLabelText("Username")).toBeInTheDocument();
    expect(meRequestCount).toBe(2);
    expect(
      router.options.context.queryClient.getQueryData(
        authQueries.me().queryKey,
      ),
    ).toBeUndefined();
  });

  test("session expired", async () => {
    let signedIn = true;
    let meRequestCount = 0;
    server.use(
      http.get("/api/auth/session", () => {
        meRequestCount++;
        return signedIn
          ? HttpResponse.json(ADMINISTRATOR)
          : noSessionResponse();
      }),
      http.get("/api/machines", () => {
        signedIn = false;
        return noSessionResponse();
      }),
    );

    const router = renderAppAt("/machines");

    expect(await screen.findByLabelText("Username")).toBeInTheDocument();
    expect(meRequestCount).toBe(3);
    expect(
      router.options.context.queryClient.getQueryData(
        authQueries.me().queryKey,
      ),
    ).toBeUndefined();
    expect(router.state.location.pathname).toBe("/login");
    expect(router.state.location.search).toMatchObject({
      redirect: "/machines",
      expired: true,
    });
    expect(
      screen.getByText("Your session has expired. Sign in again to continue."),
    ).toBeInTheDocument();
  });

  test("replaced session", async () => {
    let meRequestCount = 0;
    server.use(
      http.get("/api/auth/session", () => {
        meRequestCount++;
        return HttpResponse.json(ADMINISTRATOR);
      }),
      http.get("/api/machines", noSessionResponse),
    );

    const router = renderAppAt("/machines");

    await waitFor(() => expect(meRequestCount).toBe(2));
    expect(router.state.location.pathname).toBe("/machines");
  });
});
