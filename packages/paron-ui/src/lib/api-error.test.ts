import { http, HttpResponse } from "msw";
import { afterEach, describe, expect, test, vi } from "vitest";

import { createClient, createConfig } from "@/generated/api/client";
import { listMachines } from "@/generated/api/sdk.gen";
import { server } from "@/test/jsdom/server";
import { errorMessage, reportableRequestId, toApiError } from "./api-error";

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("client.interceptors.error", () => {
  const client = createClient(createConfig({ baseUrl: location.origin }));
  client.interceptors.error.use(toApiError);

  test("ApiError", async () => {
    const errorBody = { status: 403, detail: "not yours" };
    server.use(
      http.get("/api/machines", () =>
        HttpResponse.json(errorBody, { status: 403 }),
      ),
    );

    expect((await listMachines({ client })).error).toEqual(errorBody);
  });

  // A plain `Response` has no `statusText`, as over HTTP/2. `HttpResponse` of msw always sets one.
  // msw drops the default `Content-Type` of a string body, so each test sets it.
  test("text/plain", async () => {
    const plainTextBody =
      "upstream connect error or disconnect/reset before headers. reset reason: connection failure";
    server.use(
      http.get(
        "/api/machines",
        () =>
          new Response(plainTextBody, {
            status: 502,
            headers: { "content-type": "text/plain" },
          }),
      ),
    );

    expect((await listMachines({ client })).error).toEqual({
      status: 502,
      detail: plainTextBody,
    });
  });

  test("text/html", async () => {
    server.use(
      http.get(
        "/api/machines",
        () =>
          new Response("<html><body>502 Bad Gateway</body></html>", {
            status: 502,
            headers: { "content-type": "text/html" },
          }),
      ),
    );

    expect((await listMachines({ client })).error).toEqual({
      status: 502,
    });
  });

  test("empty text/plain", async () => {
    server.use(
      http.get(
        "/api/machines",
        () =>
          new Response("", {
            status: 503,
            headers: { "content-type": "text/plain" },
          }),
      ),
    );

    expect((await listMachines({ client })).error).toEqual({
      status: 503,
    });
  });

  test("no Content-Type", async () => {
    const plainTextBytes = new TextEncoder().encode(
      "Too Many Requests! Wait for 3s",
    );
    server.use(
      http.get(
        "/api/machines",
        () => new Response(plainTextBytes, { status: 429 }),
      ),
    );

    expect((await listMachines({ client })).error).toEqual({
      status: 429,
    });
  });

  test("Retry-After: seconds", async () => {
    server.use(
      http.get(
        "/api/machines",
        () =>
          new Response("Too Many Requests! Wait for 3s", {
            status: 429,
            headers: { "retry-after": "3" },
          }),
      ),
    );

    expect((await listMachines({ client })).error).toMatchObject({
      retryAfterMs: 3000,
    });
  });

  test("Retry-After: HTTP-date", async () => {
    vi.useFakeTimers({ now: Date.UTC(2026, 0, 1), toFake: ["Date"] });
    server.use(
      http.get("/api/machines", () =>
        HttpResponse.json(
          { status: 503 },
          {
            status: 503,
            headers: { "retry-after": "Thu, 01 Jan 2026 00:00:05 GMT" },
          },
        ),
      ),
    );

    expect((await listMachines({ client })).error).toMatchObject({
      retryAfterMs: 5000,
    });
  });

  test("Retry-After: invalid", async () => {
    server.use(
      http.get(
        "/api/machines",
        () =>
          new Response("slow down", {
            status: 429,
            headers: { "retry-after": "1e3" },
          }),
      ),
    );

    expect((await listMachines({ client })).error).toMatchObject({
      retryAfterMs: undefined,
    });
  });

  test("X-Request-Id", async () => {
    server.use(
      http.get("/api/machines", () =>
        HttpResponse.json(
          { status: 500 },
          {
            status: 500,
            headers: { "x-request-id": "01926f3a-7c1e-7000-8000-000000000000" },
          },
        ),
      ),
    );

    expect((await listMachines({ client })).error).toMatchObject({
      requestId: "01926f3a-7c1e-7000-8000-000000000000",
    });
  });

  test("no response", async () => {
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => {});
    server.use(http.get("/api/machines", () => HttpResponse.error()));

    expect((await listMachines({ client })).error).toEqual({
      status: 0,
      aborted: false,
    });
    expect(consoleError).toHaveBeenCalledOnce();
  });

  test("aborted", async () => {
    const consoleError = vi.spyOn(console, "error");

    expect(
      (await listMachines({ client, signal: AbortSignal.abort() })).error,
    ).toEqual({ status: 0, aborted: true });
    expect(consoleError).not.toHaveBeenCalled();
  });
});

describe("errorMessage()", () => {
  test("detail", () => {
    expect(errorMessage({ status: 403, detail: "not yours" })).toBe(
      "Not yours",
    );
  });

  test("no response", () => {
    expect(errorMessage({ status: 0 })).toBe(
      "Paron could not be reached. Check your internet connection.",
    );
  });

  test("no detail", () => {
    expect(
      errorMessage({
        status: 500,
        detail: null,
      }),
    ).toBe("Unexpected error (HTTP 500).");
  });

  test("429", () => {
    expect(
      errorMessage({
        status: 429,
        detail: "too many wrong attempts, try again later",
      }),
    ).toBe("Too many requests. Wait, then try again.");
  });

  test("429 for failed attempts", () => {
    expect(
      errorMessage({ status: 429, code: "too_many_failed_attempts" }),
    ).toBe("Too many wrong attempts. Wait, then try again.");
  });
});

describe("reportableRequestId()", () => {
  const requestId = "01926f3a-7c1e-7000-8000-000000000000";

  test("5xx", () => {
    expect(
      reportableRequestId({
        status: 500,
        requestId,
      }),
    ).toBe(requestId);
  });

  test("5xx detail", () => {
    expect(
      reportableRequestId({
        status: 502,
        detail: "the machine could not be reached",
        requestId,
      }),
    ).toBeUndefined();
  });

  test("4xx", () => {
    expect(reportableRequestId({ status: 403, requestId })).toBeUndefined();
  });
});
