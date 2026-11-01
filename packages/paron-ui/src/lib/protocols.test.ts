import { describe, expect, test } from "vitest";

import { protocolName } from "./protocols";

describe("protocolName()", () => {
  test("known protocol", () => {
    expect(protocolName("ssh")).toBe("SSH");
  });

  test("unknown protocol", () => {
    expect(protocolName("gopher")).toBe("gopher");
  });
});
