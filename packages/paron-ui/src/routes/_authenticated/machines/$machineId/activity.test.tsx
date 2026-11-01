import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import { renderAppAt } from "@/test/app";
import { MACHINE } from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

describe("/machines/$machineId/activity", () => {
  test("200", async () => {
    const auditRequests: URL[] = [];
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
      http.get("/api/audit", ({ request }) => {
        auditRequests.push(new URL(request.url));
        return HttpResponse.json({ items: [] });
      }),
    );

    renderAppAt(`/machines/${MACHINE.id}/activity`);

    expect(
      await screen.findByRole("tab", { name: "Activity" }),
    ).toHaveAttribute("aria-selected", "true");
    expect(
      screen.queryByRole("button", { name: "Filter by" }),
    ).not.toBeInTheDocument();
    expect(auditRequests.at(-1)?.searchParams.get("entity[id]")).toBe(
      MACHINE.id,
    );
  });

  test("clear filters", async () => {
    const user = userEvent.setup();
    const auditRequests: URL[] = [];
    server.use(
      http.get(`/api/machines/${MACHINE.id}`, () => HttpResponse.json(MACHINE)),
      http.get("/api/audit", ({ request }) => {
        auditRequests.push(new URL(request.url));
        return HttpResponse.json({ items: [] });
      }),
    );
    renderAppAt(`/machines/${MACHINE.id}/activity?role=actor`);

    await user.click(
      await screen.findByRole("button", { name: "Clear filters" }),
    );

    await waitFor(() =>
      expect(
        screen.getByText(/^Entries appear here/).closest('[aria-busy="true"]'),
      ).not.toBeInTheDocument(),
    );
    expect(auditRequests.at(-1)?.searchParams.get("entity[role]")).toBe(
      "actor_or_target",
    );
    expect(auditRequests.at(-1)?.searchParams.get("entity[id]")).toBe(
      MACHINE.id,
    );
  });
});
