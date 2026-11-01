import { render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";

import { AuditTimeline } from "@/features/audit/components/timeline";
import { formatTime } from "@/lib/format";
import { MACHINE_UPDATED } from "@/test/fixtures";

describe("<AuditTimeline />", () => {
  test("details", async () => {
    const user = userEvent.setup();
    render(
      <AuditTimeline
        auditEntries={{
          entries: [
            MACHINE_UPDATED,
            {
              ...MACHINE_UPDATED,
              id: "018f0000-0000-7000-8000-0000000000a2",
              created_at: "2026-10-08T11:00:00.000Z",
            },
          ],
          moreEntries: false,
          loadingMore: false,
          loadMoreFailed: false,
          loadMore: () => {},
        }}
        entityActions={() => ({})}
      />,
    );
    const newerEntry = await screen.findByRole("article", {
      name: `${formatTime("2026-10-08T12:00:00.000Z")} admin changed machine web-01`,
    });
    const olderEntry = screen.getByRole("article", {
      name: `${formatTime("2026-10-08T11:00:00.000Z")} admin changed machine web-01`,
    });

    await user.click(
      within(newerEntry).getByRole("button", { name: "Show details" }),
    );

    const hostChange = screen.getByRole("row", {
      name: "Host 10.0.0.5 10.0.0.6",
    });
    await waitFor(() =>
      expect(olderEntry.getBoundingClientRect().top).toBeGreaterThan(
        hostChange.getBoundingClientRect().bottom,
      ),
    );
  });

  test("keyboard", async () => {
    const user = userEvent.setup();
    render(
      <AuditTimeline
        auditEntries={{
          entries: Array.from({ length: 30 }, (_, index) => ({
            ...MACHINE_UPDATED,
            id: `018f0000-0000-7000-8000-${String(index).padStart(12, "0")}`,
            created_at: `2026-10-08T12:${String(59 - index)}:00.000Z`,
          })),
          moreEntries: false,
          loadingMore: false,
          loadMoreFailed: false,
          loadMore: () => {},
        }}
        entityActions={() => ({})}
      />,
    );
    const firstEntry = await screen.findByRole("article", {
      name: `${formatTime("2026-10-08T12:59:00.000Z")} admin changed machine web-01`,
    });
    const twentySixthEntry = page.getByRole("article", {
      name: `${formatTime("2026-10-08T12:34:00.000Z")} admin changed machine web-01`,
    });
    await expect.element(twentySixthEntry).not.toBeInTheDocument();

    await user.click(
      within(firstEntry).getByText(formatTime("2026-10-08T12:59:00.000Z")),
    );
    await user.keyboard("{PageDown}".repeat(25));

    await expect.element(twentySixthEntry).toHaveFocus();
    await expect.element(twentySixthEntry).toBeInViewport({ ratio: 1 });
    await expect
      .element(page.getByRole("button", { name: "Back to top" }))
      .toBeVisible();

    await user.keyboard("{PageUp}");

    await expect
      .element(
        page.getByRole("article", {
          name: `${formatTime("2026-10-08T12:35:00.000Z")} admin changed machine web-01`,
        }),
      )
      .toHaveFocus();
  });
});
