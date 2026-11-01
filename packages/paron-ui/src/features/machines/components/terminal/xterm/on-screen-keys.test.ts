import { Terminal } from "@xterm/xterm";
import { describe, expect, test } from "vitest";

import { NO_MODIFIERS } from "./key-encoding";
import { onScreenKeysAddon } from "./on-screen-keys";

function terminalWithOnScreenKeys() {
  const terminal = new Terminal();
  const onScreenKeys = onScreenKeysAddon();
  terminal.loadAddon(onScreenKeys);

  const sentData: string[] = [];
  terminal.onData((data) => sentData.push(data));

  return { terminal, onScreenKeys, sentData };
}

describe("onScreenKeysAddon()", () => {
  test("typed key", () => {
    const { terminal, sentData } = terminalWithOnScreenKeys();

    terminal.input("c");

    expect(sentData).toEqual(["c"]);
  });

  test("Ctrl+c", () => {
    const { terminal, onScreenKeys, sentData } = terminalWithOnScreenKeys();

    onScreenKeys.press("ctrl");
    terminal.input("c");
    terminal.input("c");

    expect(sentData).toEqual(["\x03", "c"]);
    expect(onScreenKeys.modifiers).toEqual(NO_MODIFIERS);
  });

  test("Ctrl+ArrowLeft", () => {
    const { onScreenKeys, sentData } = terminalWithOnScreenKeys();

    onScreenKeys.press("ctrl");
    onScreenKeys.press("ArrowLeft");

    expect(sentData).toEqual(["\x1b[1;5D"]);
  });

  test("on-screen key", () => {
    const { onScreenKeys, sentData } = terminalWithOnScreenKeys();

    onScreenKeys.press("Tab");

    expect(sentData).toEqual(["\t"]);
  });

  test("modifier off", () => {
    const { terminal, onScreenKeys, sentData } = terminalWithOnScreenKeys();

    onScreenKeys.press("alt");
    onScreenKeys.press("alt");
    terminal.input("b");

    expect(sentData).toEqual(["b"]);
  });

  test("terminal report", () => {
    const { terminal, onScreenKeys, sentData } = terminalWithOnScreenKeys();

    onScreenKeys.press("ctrl");
    terminal.input("\x1b[I");
    terminal.input("c");

    expect(sentData).toEqual(["\x1b[I", "\x03"]);
  });

  test("change listener", () => {
    const { terminal, onScreenKeys } = terminalWithOnScreenKeys();
    let changeCount = 0;
    onScreenKeys.onChange(() => changeCount++);

    onScreenKeys.press("ctrl");
    terminal.input("c");

    expect(changeCount).toBe(2);
  });

  test("dispose", () => {
    const { terminal, onScreenKeys } = terminalWithOnScreenKeys();
    onScreenKeys.press("ctrl");

    onScreenKeys.dispose();
    const dataAfterDispose: string[] = [];
    terminal.onData((data) => dataAfterDispose.push(data));
    terminal.input("c");

    expect(dataAfterDispose).toEqual(["c"]);
  });
});
