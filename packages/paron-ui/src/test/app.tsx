import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { render } from "@testing-library/react";
import type { ReactNode } from "react";

import { App, createApp } from "@/app";
import { Route as RootRoute } from "@/routes/__root";

export function renderAppAt(path: string) {
  const app = createApp(createMemoryHistory({ initialEntries: [path] }));
  render(<App {...app} />);

  return app.router;
}

// For a component that needs a router: the page is at "/", and every other path shows "Other page".
export function renderInRouter(page: ReactNode) {
  const root = createRootRoute({ component: RootRoute.options.component });
  const router = createRouter({
    routeTree: root.addChildren([
      createRoute({
        getParentRoute: () => root,
        path: "/",
        component: () => page,
      }),
      createRoute({
        getParentRoute: () => root,
        path: "$",
        component: () => <p>Other page</p>,
      }),
    ]),
    history: createMemoryHistory(),
  });
  render(<RouterProvider router={router} />);

  return router;
}
