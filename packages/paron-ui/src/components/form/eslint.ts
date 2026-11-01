import type { Linter } from "eslint";

// Loaded by the root eslint.config.ts. It uses the typescript-eslint copy of
// no-restricted-imports: the dialog folder already sets the core rule, and a
// second block for the same files would replace that one instead of adding to it.
export const formImports = {
  files: ["src/**/*.{ts,tsx}"],
  ignores: ["src/components/form/**"],
  rules: {
    "@typescript-eslint/no-restricted-imports": [
      "error",
      {
        paths: [
          {
            name: "@tanstack/react-form",
            importNames: ["useForm"],
            message:
              "Build forms with useAppForm from @/components/form, so every form shares the same field components.",
          },
        ],
      },
    ],
  },
} satisfies Linter.Config;
