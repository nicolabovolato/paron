import { heyApiPlugin } from "@hey-api/vite-plugin";
import { paraglideVitePlugin } from "@inlang/paraglide-js";
import tailwindcss from "@tailwindcss/vite";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import react from "@vitejs/plugin-react";
import { playwright } from "@vitest/browser-playwright";
import browserslistToEsbuild from "browserslist-to-esbuild";
import { resolve } from "node:path";
import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";

const PORT = Number(process.env.JS_FRONTEND_EMBED_PORT ?? 5173);
const OUT_DIR = process.env.JS_FRONTEND_EMBED_DIST ?? "dist";
const OPENAPI_PATH = resolve(process.env.OPENAPI_PATH ?? "./openapi.json");
const VERSION = process.env.PARON_VERSION ?? "0.0.0";

export default defineConfig({
  define: {
    __PARON_VERSION__: JSON.stringify(VERSION),
  },

  plugins: [
    paraglideVitePlugin({
      project: "./i18n.inlang",
      outdir: "./src/generated/paraglide",
      outputStructure: "message-modules",
      strategy: ["baseLocale"],
      emitTsDeclarations: true,
      emitGitIgnore: false,
      emitPrettierIgnore: false,
      emitReadme: false,
    }),
    heyApiPlugin({
      config: {
        input: OPENAPI_PATH,
        output: "./src/generated/api",
        plugins: [
          { name: "@tanstack/react-query", mutationKeys: true },
          { name: "zod", metadata: true },
        ],
        logs: { file: false, level: "silent" },
      },
    }),
    tanstackRouter({
      target: "react",
      routesDirectory: "./src/routes",
      generatedRouteTree: "./src/generated/routeTree.gen.ts",
      routeFileIgnorePattern: "\\.test\\.tsx?$",
      autoCodeSplitting: !process.env.VITEST,
    }),
    react(),
    tailwindcss(),
  ],

  resolve: {
    tsconfigPaths: true,
  },

  server: {
    port: PORT,
    strictPort: true,
  },

  build: {
    target: browserslistToEsbuild(),
    outDir: OUT_DIR,
    emptyOutDir: true,
    manifest: true,
  },

  test: {
    projects: [
      {
        test: {
          name: "unit",
          environment: "jsdom",
          environmentOptions: { jsdom: { url: "https://paron.test/" } },
          setupFiles: ["./src/test/jsdom/setup.ts"],
          exclude: [...configDefaults.exclude, "**/*.browser.test.*"],
        },
      },
      {
        test: {
          name: "browser",
          include: ["src/**/*.browser.test.{ts,tsx}"],
          setupFiles: ["./src/test/browser/setup.ts"],
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            instances: [{ browser: "chromium" }, { browser: "webkit" }],
          },
        },
      },
    ],
  },
});
