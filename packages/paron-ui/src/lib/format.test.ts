import { describe, expect, test } from "vitest";

import { formatRelative } from "./format";

const NOW = Date.UTC(2026, 0, 1);
const MINUTE = 60;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

describe("formatRelative()", () => {
  test.each([
    ["now", 0],
    ["now", -0.4],
    ["59 seconds ago", -59.9],
    ["1 minute ago", -MINUTE],
    ["1 minute ago", -119],
    ["59 minutes ago", -(HOUR - 1)],
    ["1 hour ago", -HOUR],
    ["23 hours ago", -(DAY - 1)],
    ["yesterday", -DAY],
    ["6 days ago", -(7 * DAY - 1)],
    ["last week", -7 * DAY],
    ["4 weeks ago", -30 * DAY],
    ["last month", -31 * DAY],
    ["11 months ago", -364 * DAY],
    ["last year", -365 * DAY],
    ["in 59 seconds", 59.9],
    ["in 59 minutes", HOUR - 1],
    ["in 1 hour", HOUR],
    ["tomorrow", DAY],
  ])("%s", (relativeTime, seconds) => {
    const timestamp = new Date(NOW + seconds * 1000).toISOString();

    expect(formatRelative(timestamp, NOW)).toBe(relativeTime);
  });
});
