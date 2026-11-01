import { describe, expect, test } from "vitest";

const catalogues = import.meta.glob<Record<string, string>>(
  "/src/**/i18n/en.json",
  {
    eager: true,
    import: "default",
  },
);
const sourceFiles = import.meta.glob<string>(
  ["/src/**/*.{ts,tsx}", "!/src/generated/**"],
  {
    eager: true,
    query: "?raw",
    import: "default",
  },
);

const usedKeys = new Set(
  Object.values(sourceFiles).flatMap((sourceFile) =>
    [...sourceFile.matchAll(/\bm\.(\w+)/g)].map(([, key]) => key),
  ),
);
const catalogueKeys = Object.values(catalogues)
  .flatMap((catalogue) => Object.keys(catalogue))
  .filter((key) => key !== "$schema");

describe("en.json", () => {
  test.each(catalogueKeys)("%s", (key) => {
    expect(usedKeys).toContain(key);
  });
});
