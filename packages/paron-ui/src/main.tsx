import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import "./styles.css";

import { App, createApp } from "@/app";
import { client } from "@/generated/api/client.gen";
import { toApiError } from "@/lib/api-error";

client.interceptors.error.use(toApiError);

// eslint-disable-next-line @typescript-eslint/no-non-null-assertion
const rootElement = document.getElementById("root")!;

if (!rootElement.innerHTML) {
  createRoot(rootElement).render(
    <StrictMode>
      <App {...createApp()} />
    </StrictMode>,
  );
}
