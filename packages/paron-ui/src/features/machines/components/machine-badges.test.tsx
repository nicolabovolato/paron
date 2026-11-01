import { render, screen } from "@testing-library/react";
import { describe, expect, test } from "vitest";

import type { ConnectivityResponse } from "@/generated/api/types.gen";
import { ClockProvider } from "@/hooks/now";

import { MachineConnectionBadge } from "./machine-badges";

const HOUR_MS = 3_600_000;

describe("<MachineConnectionBadge />", () => {
  test.each([
    ["invited", HOUR_MS, "Setup pending"],
    ["expired", -HOUR_MS, "Token expired"],
  ])("%s", (_, expiresInMs, badgeText) => {
    const connectivity = {
      kind: "tunneled",
      enrollment: {
        state: "invited",
        expires_at: new Date(Date.now() + expiresInMs).toISOString(),
      },
    } satisfies ConnectivityResponse;

    render(<MachineConnectionBadge connectivity={connectivity} />, {
      wrapper: ClockProvider,
    });

    expect(screen.getByText(badgeText)).toBeInTheDocument();
  });
});
