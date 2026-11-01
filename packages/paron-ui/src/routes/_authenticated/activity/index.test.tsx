import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { renderAppAt } from "@/test/app";
import {
  ADMINISTRATOR,
  administratorWithout,
  MACHINE,
  MACHINE_UPDATED,
} from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/activity", () => {
  test("200", async () => {
    const user = userEvent.setup();
    server.use(
      http.get("/api/audit", () =>
        HttpResponse.json({ items: [MACHINE_UPDATED] }),
      ),
    );
    renderAppAt("/activity");

    expect(
      await screen.findByRole("heading", { name: "Thursday, October 8, 2026" }),
    ).toBeInTheDocument();
    expect(screen.getByText("No more entries")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show details" }));

    expect(
      screen.getByRole("row", { name: "Host 10.0.0.5 10.0.0.6" }),
    ).toBeInTheDocument();
  });

  test("entity filter", async () => {
    const user = userEvent.setup();
    const auditRequests: URL[] = [];
    server.use(
      http.get("/api/audit", ({ request }) => {
        auditRequests.push(new URL(request.url));
        return HttpResponse.json({ items: [MACHINE_UPDATED] });
      }),
    );
    renderAppAt("/activity");

    await user.click(await screen.findByRole("button", { name: "web-01" }));
    await user.click(
      await screen.findByRole("menuitem", { name: "Filter by web-01" }),
    );

    expect(
      await screen.findByRole("button", { name: "Machine web-01" }),
    ).toBeInTheDocument();
    expect(auditRequests.at(-1)?.searchParams.get("entity[id]")).toBe(
      MACHINE.id,
    );
    expect(auditRequests.at(-1)?.searchParams.get("entity[role]")).toBe(
      "actor_or_target",
    );

    await user.click(
      screen.getByRole("button", { name: "Affected", pressed: false }),
    );

    await waitFor(() =>
      expect(
        screen.getByRole("feed").closest('[aria-busy="true"]'),
      ).not.toBeInTheDocument(),
    );
    expect(auditRequests.at(-1)?.searchParams.get("entity[role]")).toBe(
      "target",
    );
  });

  test("picked filter", async () => {
    const user = userEvent.setup();
    const auditRequests: URL[] = [];
    server.use(
      http.get("/api/users", () =>
        HttpResponse.json({ items: [ADMINISTRATOR], total: 1 }),
      ),
      http.get("/api/machines", () =>
        HttpResponse.json({ items: [MACHINE], total: 1 }),
      ),
      http.get("/api/audit", ({ request }) => {
        auditRequests.push(new URL(request.url));
        return HttpResponse.json({ items: [MACHINE_UPDATED] });
      }),
    );
    renderAppAt("/activity");

    await user.click(await screen.findByRole("button", { name: "Filter by" }));
    await user.click(await screen.findByRole("button", { name: "Machine" }));
    await user.click(await screen.findByRole("option", { name: MACHINE.name }));

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: `Machine ${MACHINE.name}` }),
      ).toHaveFocus(),
    );
    expect(auditRequests.at(-1)?.searchParams.get("entity[id]")).toBe(
      MACHINE.id,
    );

    await user.click(
      screen.getByRole("button", { name: "Remove this filter" }),
    );

    await waitFor(() =>
      expect(
        screen.getByRole("feed").closest('[aria-busy="true"]'),
      ).not.toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Filter by" })).toHaveFocus();
  });

  test("pending", async () => {
    const user = userEvent.setup();
    let sendOldestFirst = () => {};
    const oldestFirstSent = new Promise<void>((resolve) => {
      sendOldestFirst = resolve;
    });
    server.use(
      http.get("/api/audit", async ({ request }) => {
        if (new URL(request.url).searchParams.get("order") === "oldest_first")
          await oldestFirstSent;
        return HttpResponse.json({ items: [MACHINE_UPDATED] });
      }),
    );
    renderAppAt("/activity");

    await user.click(
      await screen.findByRole("button", { name: "Newest first" }),
    );

    expect(
      await screen.findByRole("button", { name: "Oldest first" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("feed").closest('[aria-busy="true"]'),
    ).toBeInTheDocument();

    sendOldestFirst();

    await waitFor(() =>
      expect(
        screen.getByRole("feed").closest('[aria-busy="true"]'),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: "Oldest first" }),
    ).toBeInTheDocument();
  });

  test("no user:read", async () => {
    const user = userEvent.setup();
    server.use(
      http.get("/api/auth/session", () =>
        HttpResponse.json(administratorWithout("user:read")),
      ),
      http.get("/api/audit", () =>
        HttpResponse.json({ items: [MACHINE_UPDATED] }),
      ),
    );
    renderAppAt("/activity");

    await user.click(await screen.findByRole("button", { name: "admin" }));

    expect(
      await screen.findByRole("menuitem", { name: "Filter by admin" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("menuitem", { name: "Go to admin" }),
    ).not.toBeInTheDocument();

    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "web-01" }));

    expect(
      await screen.findByRole("menuitem", { name: "Go to web-01" }),
    ).toBeInTheDocument();
  });

  test("no match", async () => {
    server.use(http.get("/api/audit", () => HttpResponse.json({ items: [] })));

    renderAppAt(
      `/activity?entity=${encodeURIComponent(JSON.stringify({ type: "user", id: ADMINISTRATOR.id, name: "admin" }))}`,
    );

    expect(
      await screen.findByText("No entry matches these filters."),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Clear filters" }),
    ).toBeInTheDocument();
  });
});
