import { act, render } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";

import { ClockProvider, useNow } from "./now";

// React commits between timers, so time moves in small steps, each in its own act().
async function advanceTimersBy(milliseconds: number) {
  for (let elapsed = 0; elapsed < milliseconds; elapsed += 100)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(100);
    });
}

afterEach(() => {
  vi.useRealTimers();
});

describe("useNow()", () => {
  test("selection changed", async () => {
    vi.useFakeTimers({ now: 0 });
    const renderCounts = { everySecond: 0, everyFiveSeconds: 0 };

    function EverySecond() {
      renderCounts.everySecond++;
      useNow((now) => Math.floor(now / 1000));
      return null;
    }

    function EveryFiveSeconds() {
      renderCounts.everyFiveSeconds++;
      useNow((now) => Math.floor(now / 5000));
      return null;
    }

    // StrictMode doubles the renders that this test counts.
    render(
      <>
        <EverySecond />
        <EveryFiveSeconds />
      </>,
      { reactStrictMode: false, wrapper: ClockProvider },
    );

    await advanceTimersBy(4500);
    expect(renderCounts).toEqual({ everySecond: 5, everyFiveSeconds: 1 });

    await advanceTimersBy(600);
    expect(renderCounts).toEqual({ everySecond: 6, everyFiveSeconds: 2 });
  });

  test("remounted", () => {
    vi.useFakeTimers({ now: 0 });
    const renderedSeconds: number[] = [];

    function Seconds() {
      renderedSeconds.push(useNow((now) => Math.floor(now / 1000)));
      return null;
    }

    render(<Seconds />, {
      reactStrictMode: false,
      wrapper: ClockProvider,
    }).unmount();
    vi.setSystemTime(60_000);
    render(<Seconds />, { reactStrictMode: false, wrapper: ClockProvider });

    expect(renderedSeconds).toEqual([0, 60]);
  });
});
