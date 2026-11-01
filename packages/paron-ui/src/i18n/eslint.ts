import type { Linter } from "eslint";
import i18next from "eslint-plugin-i18next";

// Loaded by the root eslint.config.ts. Checks JSX text and the attributes that
// show text to a person; every other attribute holds code, such as a variant or a route.
export const literalText = {
  files: ["src/**/*.tsx"],
  ignores: ["src/**/*.test.tsx", "src/test/**"],
  plugins: { i18next },
  rules: {
    "i18next/no-literal-string": [
      "error",
      {
        mode: "jsx-only",
        "jsx-attributes": {
          include: [
            "aria-label",
            "alt",
            "title",
            "placeholder",
            "label",
            "description",
            "confirmLabel",
            "errorTitle",
          ],
          exclude: [],
        },
        message:
          "Put UI text in src/i18n/en.json or the feature's i18n/en.json and use m.<key>().",
      },
    ],
  },
} satisfies Linter.Config;
