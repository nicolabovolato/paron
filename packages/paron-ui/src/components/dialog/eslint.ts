import type { Linter } from "eslint";

const OPEN_WITH_USE_DIALOG =
  "Open dialogs with useDialog() from @/components/dialog. It nests them the way Base UI needs: backdrop, focus and Escape.";

// Loaded by the root eslint.config.ts. It must not be named eslint.config.ts:
// ESLint would then use it instead of the root config for this folder.
export const dialogImports = {
  files: ["src/**/*.{ts,tsx}"],
  ignores: ["src/components/dialog/**"],
  rules: {
    "no-restricted-imports": [
      "error",
      {
        paths: [
          { name: "@base-ui/react/dialog", message: OPEN_WITH_USE_DIALOG },
          {
            name: "@base-ui/react/alert-dialog",
            message: OPEN_WITH_USE_DIALOG,
          },
          {
            name: "@/components/shadcn/dialog",
            importNames: [
              "Dialog",
              "DialogTrigger",
              "DialogPortal",
              "DialogOverlay",
              "DialogContent",
            ],
            message: OPEN_WITH_USE_DIALOG,
          },
          {
            name: "@/components/shadcn/alert-dialog",
            importNames: [
              "AlertDialog",
              "AlertDialogTrigger",
              "AlertDialogPortal",
              "AlertDialogOverlay",
              "AlertDialogContent",
            ],
            message: OPEN_WITH_USE_DIALOG,
          },
        ],
        patterns: [
          {
            regex: "^@/components/dialog/",
            message:
              "Import from @/components/dialog. The files inside the folder are private.",
          },
        ],
      },
    ],
  },
} satisfies Linter.Config;
