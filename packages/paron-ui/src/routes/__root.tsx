import type { QueryClient } from "@tanstack/react-query";
import { createRootRouteWithContext, Outlet } from "@tanstack/react-router";
import { useState } from "react";

import { DialogProvider } from "@/components/dialog";
import { createToastManager, Toaster } from "@/components/shadcn/toast";

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RouteComponent,
});

function RouteComponent() {
  const [toastManager] = useState(createToastManager);

  return (
    <Toaster toastManager={toastManager}>
      <DialogProvider>
        <Outlet />
      </DialogProvider>
    </Toaster>
  );
}
