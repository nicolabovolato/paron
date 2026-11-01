import "@testing-library/jest-dom/vitest";

import { cleanup, configure } from "@testing-library/react";
import { afterAll, afterEach, beforeAll } from "vitest";

import { client } from "@/generated/api/client.gen";
import { toApiError } from "@/lib/api-error";
import { server } from "./server";

client.setConfig({ baseUrl: location.origin });
client.interceptors.error.use(toApiError);

// As the app in development: main.tsx renders under StrictMode.
configure({ reactStrictMode: true });

// jsdom has no layout and no media queries.
window.scrollTo = () => {};
document.elementFromPoint = () => null;
window.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
window.matchMedia = (query) => ({
  matches: false,
  media: query,
  onchange: null,
  addEventListener: () => {},
  removeEventListener: () => {},
  // xterm.js 6.0 still calls these.
  // eslint-disable-next-line @typescript-eslint/no-deprecated
  addListener: () => {},
  // eslint-disable-next-line @typescript-eslint/no-deprecated
  removeListener: () => {},
  dispatchEvent: () => false,
});

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));

afterEach(() => {
  server.resetHandlers();
  cleanup();
});

afterAll(() => server.close());
