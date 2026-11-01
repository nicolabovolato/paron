import { MutationObserver, QueryObserver } from "@tanstack/react-query";
import { screen, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createApp } from "@/app";
import {
  listMachinesOptions,
  testMachineConnectionMutation,
} from "@/generated/api/@tanstack/react-query.gen";
import { listMachines } from "@/generated/api/sdk.gen";
import { renderAppAt } from "@/test/app";
import {
  administratorWithout,
  EMPTY_PAGE,
  errorResponse,
  MACHINE,
} from "@/test/fixtures";
import { server } from "@/test/jsdom/server";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("createApp()", () => {
  describe("queries", () => {
    test("429 Retry-After: 0", async () => {
      vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
      let requestCount = 0;
      server.use(
        http.get("/api/machines", () =>
          ++requestCount === 1
            ? new Response("Too Many Requests! Wait for 0s", {
                status: 429,
                headers: { "retry-after": "0" },
              })
            : HttpResponse.json(EMPTY_PAGE),
        ),
      );
      const { queryClient } = createApp();

      const machines = queryClient.query(listMachinesOptions());
      await vi.advanceTimersByTimeAsync(999);
      const requestCountBeforeOneSecond = requestCount;
      await vi.advanceTimersByTimeAsync(1);

      expect(requestCountBeforeOneSecond).toBe(1);
      expect(requestCount).toBe(2);
      await expect(machines).resolves.toEqual(EMPTY_PAGE);
    });

    test("429 Retry-After: 3600", async () => {
      let requestCount = 0;
      server.use(
        http.get("/api/machines", () => {
          requestCount++;
          return new Response("come back later", {
            status: 429,
            headers: { "retry-after": "3600" },
          });
        }),
      );
      const { queryClient } = createApp();

      await expect(
        queryClient.query(listMachinesOptions()),
      ).rejects.toMatchObject({ status: 429 });
      expect(requestCount).toBe(1);
    });
  });

  describe("mutations", () => {
    test("429", async () => {
      let requestCount = 0;
      server.use(
        http.post("/api/machines/:id/test-connection", () =>
          ++requestCount === 1
            ? new Response("slow down", {
                status: 429,
                headers: { "retry-after": "0" },
              })
            : new Response(null, { status: 204 }),
        ),
      );
      const { queryClient } = createApp();

      await new MutationObserver(
        queryClient,
        testMachineConnectionMutation(),
      ).mutate({ path: { id: MACHINE.id }, query: { protocol: "ssh" } });

      expect(requestCount).toBe(2);
    });

    test("refetched", async () => {
      let listRequestCount = 0;
      server.use(
        http.get("/api/machines", () => {
          listRequestCount++;
          return HttpResponse.json(EMPTY_PAGE);
        }),
        http.post(
          "/api/machines/:id/test-connection",
          () => new Response(null, { status: 204 }),
        ),
      );
      const { queryClient } = createApp();
      const unsubscribe = new QueryObserver(
        queryClient,
        listMachinesOptions(),
      ).subscribe(() => {});
      await queryClient.query(listMachinesOptions());
      expect(listRequestCount).toBe(1);

      await new MutationObserver(
        queryClient,
        testMachineConnectionMutation(),
      ).mutate({ path: { id: MACHINE.id }, query: { protocol: "ssh" } });

      expect(listRequestCount).toBe(2);
      unsubscribe();
    });

    test("invalidates: false", async () => {
      let listRequestCount = 0;
      server.use(
        http.get("/api/machines", () => {
          listRequestCount++;
          return HttpResponse.json(EMPTY_PAGE);
        }),
        http.post(
          "/api/machines/:id/test-connection",
          () => new Response(null, { status: 204 }),
        ),
      );
      const { queryClient } = createApp();
      const unsubscribe = new QueryObserver(
        queryClient,
        listMachinesOptions(),
      ).subscribe(() => {});
      await queryClient.query(listMachinesOptions());

      await new MutationObserver(queryClient, {
        ...testMachineConnectionMutation(),
        meta: { invalidates: false },
      }).mutate({ path: { id: MACHINE.id }, query: { protocol: "ssh" } });

      expect(listRequestCount).toBe(1);
      unsubscribe();
    });
  });

  describe("connection", () => {
    test("no response", async () => {
      vi.spyOn(console, "error").mockImplementation(() => {});
      server.use(
        http.post("/api/machines/:id/test-connection", () =>
          HttpResponse.error(),
        ),
        http.get("/health", () => HttpResponse.error()),
      );
      const router = renderAppAt("/does-not-exist");
      await screen.findByRole("heading", { name: "Page not found" });
      vi.useFakeTimers({
        toFake: ["setTimeout", "clearTimeout"],
        shouldAdvanceTime: true,
      });

      await expect(
        new MutationObserver(
          router.options.context.queryClient,
          testMachineConnectionMutation(),
        ).mutate({ path: { id: MACHINE.id }, query: { protocol: "ssh" } }),
      ).rejects.toMatchObject({ status: 0, aborted: false });

      expect(await screen.findByRole("alert")).toHaveTextContent(
        "No connection to Paron",
      );
      expect(
        screen.getByRole("button", { name: "Reload" }),
      ).toBeInTheDocument();
    });

    test("health answered", async () => {
      vi.spyOn(console, "error").mockImplementation(() => {});
      server.use(
        http.post("/api/machines/:id/test-connection", () =>
          HttpResponse.error(),
        ),
        http.get("/health", () => HttpResponse.error(), { once: true }),
        http.get("/health", () => new Response(null, { status: 200 })),
      );
      const router = renderAppAt("/does-not-exist");
      await screen.findByRole("heading", { name: "Page not found" });
      vi.useFakeTimers({
        toFake: ["setTimeout", "clearTimeout"],
        shouldAdvanceTime: true,
      });
      await new MutationObserver(
        router.options.context.queryClient,
        testMachineConnectionMutation(),
      )
        .mutate({ path: { id: MACHINE.id }, query: { protocol: "ssh" } })
        .catch(() => {});
      const banner = await screen.findByRole("alert");

      await vi.advanceTimersByTimeAsync(1_000);

      await waitFor(() => expect(banner).not.toBeVisible());
    });

    test("aborted", async () => {
      const router = renderAppAt("/does-not-exist");
      await screen.findByRole("heading", { name: "Page not found" });

      await expect(
        router.options.context.queryClient.query({
          queryKey: ["aborted"],
          queryFn: () =>
            listMachines({ signal: AbortSignal.abort(), throwOnError: true }),
          retry: false,
        }),
      ).rejects.toMatchObject({ status: 0, aborted: true });

      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    });
  });

  describe("router", () => {
    test("unknown path", async () => {
      renderAppAt("/does-not-exist");

      expect(
        await screen.findByRole("heading", { name: "Page not found" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("link", { name: "Activity" }),
      ).toBeInTheDocument();
    });

    test("404", async () => {
      server.use(
        http.get("/api/machines/:id", () =>
          errorResponse({
            status: 404,
            detail: "no such entity",
          }),
        ),
      );

      renderAppAt(`/machines/${MACHINE.id}`);

      expect(
        await screen.findByRole("heading", { name: "Machine not found" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("link", { name: "Go to Machines" }),
      ).toHaveAttribute("href", "/machines");
    });

    test("invalid link", async () => {
      let userRequestCount = 0;
      server.use(
        http.get("/api/users/:id", () => {
          userRequestCount++;
          return HttpResponse.json({});
        }),
      );

      renderAppAt("/users/01a0e765-0000-7000-8000-0000000000001");

      expect(
        await screen.findByRole("heading", { name: "User not found" }),
      ).toBeInTheDocument();
      expect(screen.getByRole("link", { name: "Go to Users" })).toHaveAttribute(
        "href",
        "/users",
      );
      expect(userRequestCount).toBe(0);
    });

    test("403", async () => {
      server.use(
        http.get("/api/machines", () =>
          errorResponse({
            status: 403,
            detail: "not yours",
          }),
        ),
      );

      renderAppAt("/machines");

      expect(
        await screen.findByRole("heading", { name: "No access" }),
      ).toBeInTheDocument();
    });

    test("no user:read", async () => {
      let userListRequestCount = 0;
      server.use(
        http.get("/api/auth/session", () =>
          HttpResponse.json(administratorWithout("user:read")),
        ),
        http.get("/api/users", () => {
          userListRequestCount++;
          return HttpResponse.json(EMPTY_PAGE);
        }),
      );

      renderAppAt("/users");

      expect(
        await screen.findByRole("heading", { name: "No access" }),
      ).toBeInTheDocument();
      expect(userListRequestCount).toBe(0);
    });
  });
});
