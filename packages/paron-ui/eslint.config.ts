import js from "@eslint/js";
import pluginQuery from "@tanstack/eslint-plugin-query";
import pluginRouter from "@tanstack/eslint-plugin-router";
import tailwind from "eslint-plugin-better-tailwindcss";
import boundaries from "eslint-plugin-boundaries";
import compat from "eslint-plugin-compat";
import reactHooks from "eslint-plugin-react-hooks";
import { reactRefresh } from "eslint-plugin-react-refresh";
import { defineConfig, globalIgnores } from "eslint/config";
import tseslint from "typescript-eslint";

import { dialogImports } from "./src/components/dialog/eslint";
import { formImports } from "./src/components/form/eslint";
import { literalText } from "./src/i18n/eslint";

export default defineConfig([
  globalIgnores(["dist", "src/generated", "src/components/shadcn"]),
  {
    files: ["**/*.{ts,tsx}"],
    extends: [
      js.configs.recommended,
      tseslint.configs.strictTypeChecked,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite({ extraHOCs: ["withFieldGroup"] }),
      pluginQuery.configs["flat/recommended-strict"],
      pluginRouter.configs["flat/recommended"],
      compat.configs["flat/recommended"],
    ],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      "@typescript-eslint/switch-exhaustiveness-check": [
        "error",
        { considerDefaultExhaustiveForUnions: true },
      ],
      "@typescript-eslint/no-confusing-void-expression": [
        "error",
        { ignoreArrowShorthand: true },
      ],
      "@typescript-eslint/restrict-template-expressions": [
        "error",
        { allowNumber: true },
      ],
      "@typescript-eslint/only-throw-error": [
        "error",
        {
          allow: [
            {
              from: "package",
              package: "@tanstack/router-core",
              name: "Redirect",
            },
            {
              from: "package",
              package: "@tanstack/router-core",
              name: "NotFoundError",
            },
          ],
        },
      ],
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { boundaries },
    settings: {
      "import/resolver": { typescript: { alwaysTryTypes: true } },
      "boundaries/legacy-templates": false,
      "boundaries/elements": [
        { type: "shared", pattern: ["src/components", "src/hooks", "src/lib"] },
        { type: "feature", pattern: "src/features/*", capture: ["name"] },
        { type: "routes", pattern: "src/routes" },
      ],
    },
    rules: {
      "boundaries/dependencies": [
        "error",
        {
          default: "allow",
          checkAllOrigins: true,
          policies: [
            {
              from: { element: { type: "shared" } },
              disallow: {
                to: { element: { types: { anyOf: ["feature", "routes"] } } },
              },
              message:
                "Shared code must not depend on a feature or on the app layer.",
            },
            {
              from: { element: { type: "feature" } },
              disallow: { to: { element: { type: "routes" } } },
              message: "A feature must not depend on the app layer.",
            },
            {
              from: { element: { type: "feature" } },
              disallow: {
                to: {
                  element: {
                    type: "feature",
                    captured: { name: "!{{ from.element.captured.name }}" },
                  },
                },
              },
              message:
                "A feature must not import another feature; compose them in a route.",
            },
            {
              from: {
                element: { types: { anyOf: ["shared", "feature"] } },
                // A row link and the route-level navigation UI only follow a
                // click to a destination their caller gave them. Tests stand in
                // for the routes.
                file: {
                  path: "!**/{src/components/route/*.tsx,src/components/table.tsx,*.test.ts,*.test.tsx}",
                },
              },
              disallow: {
                to: {
                  module: {
                    origin: "external",
                    source: "@tanstack/react-router",
                  },
                },
                dependency: {
                  specifiers: [
                    "useNavigate",
                    "useRouter",
                    "useMatchRoute",
                    "useCanGoBack",
                    "redirect",
                  ],
                },
              },
              message:
                "Navigation is decided in routes/. Take a callback, or a link from the caller.",
            },
            {
              disallow: {
                to: { file: { path: "**/src/components/shadcn/toast.tsx" } },
                dependency: { specifiers: ["toast"] },
              },
              message:
                "Use useToastManager(). The module-level toast manager is shared by every App.",
            },
          ],
        },
      ],
    },
  },
  dialogImports,
  formImports,
  literalText,
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [tailwind.configs["recommended-error"]],
    settings: { "better-tailwindcss": { entryPoint: "src/styles.css" } },
    rules: {
      // Prettier decides where a line breaks. Its Tailwind plugin sorts the classes on save, and
      // the order rule of this plugin checks the same order: both read src/styles.css.
      "better-tailwindcss/enforce-consistent-line-wrapping": "off",
    },
  },
  {
    files: ["src/routes/**/*.tsx"],
    rules: {
      "react-refresh/only-export-components": "off",
    },
  },
]);
