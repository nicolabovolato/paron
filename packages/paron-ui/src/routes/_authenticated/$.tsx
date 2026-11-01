import { createFileRoute, notFound } from "@tanstack/react-router";

export const Route = createFileRoute("/_authenticated/$")({
  loader: () => {
    throw notFound();
  },
});
