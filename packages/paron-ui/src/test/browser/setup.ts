import "@testing-library/jest-dom/vitest";

import "@/styles.css";

import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";

// As the app in development: main.tsx renders under StrictMode.
configure({ reactStrictMode: true });

afterEach(() => cleanup());
