import { describe, expect, test } from "vitest";

import {
  isTerminalReport,
  keySequence,
  NO_MODIFIERS,
  withModifiers,
} from "./key-encoding";

const CTRL = { ctrl: true, alt: false };
const ALT = { ctrl: false, alt: true };
const CTRL_ALT = { ctrl: true, alt: true };

describe("keySequence()", () => {
  test("cursor keys", () => {
    expect(keySequence("ArrowUp", false)).toBe("\x1b[A");
    expect(keySequence("ArrowUp", true)).toBe("\x1bOA");
    expect(keySequence("Home", false)).toBe("\x1b[H");
  });

  test("function keys", () => {
    expect(keySequence("F1", false)).toBe("\x1bOP");
    expect(keySequence("F5", false)).toBe("\x1b[15~");
    expect(keySequence("PageDown", true)).toBe("\x1b[6~");
  });

  test("Escape and Tab", () => {
    expect(keySequence("Escape", false)).toBe("\x1b");
    expect(keySequence("Tab", false)).toBe("\t");
  });
});

describe("isTerminalReport()", () => {
  test("true", () => {
    expect(isTerminalReport("\x1b[I")).toBe(true);
    expect(isTerminalReport("\x1b[<0;10;5M")).toBe(true);
    expect(isTerminalReport("\x1b[12;40R")).toBe(true);
    expect(isTerminalReport("\x1b[200~ls\x1b[201~")).toBe(true);
  });

  test("false", () => {
    expect(isTerminalReport("c")).toBe(false);
    expect(isTerminalReport("\x1b")).toBe(false);
    expect(isTerminalReport("\x1b[D")).toBe(false);
    expect(isTerminalReport("\x1bOP")).toBe(false);
    expect(isTerminalReport("\x1b[15~")).toBe(false);
    expect(isTerminalReport("hello ")).toBe(false);
  });
});

describe("withModifiers()", () => {
  test("no modifier", () => {
    expect(withModifiers("c", NO_MODIFIERS)).toBe("c");
    expect(withModifiers("\x1b[A", NO_MODIFIERS)).toBe("\x1b[A");
  });

  test("Ctrl", () => {
    expect(withModifiers("c", CTRL)).toBe("\x03");
    expect(withModifiers("C", CTRL)).toBe("\x03");
    expect(withModifiers("[", CTRL)).toBe("\x1b");
    expect(withModifiers(" ", CTRL)).toBe("\x00");
    expect(withModifiers("?", CTRL)).toBe("\x7f");
    expect(withModifiers("1", CTRL)).toBe("1");
  });

  test("Alt", () => {
    expect(withModifiers("b", ALT)).toBe("\x1bb");
  });

  test("Ctrl+Alt", () => {
    expect(withModifiers("x", CTRL_ALT)).toBe("\x1b\x18");
  });

  test("cursor keys", () => {
    expect(withModifiers("\x1b[D", CTRL)).toBe("\x1b[1;5D");
    expect(withModifiers("\x1bOD", CTRL)).toBe("\x1b[1;5D");
    expect(withModifiers("\x1b[F", ALT)).toBe("\x1b[1;3F");
  });

  test("function keys", () => {
    expect(withModifiers("\x1bOP", CTRL)).toBe("\x1b[1;5P");
    expect(withModifiers("\x1b[5~", ALT)).toBe("\x1b[5;3~");
    expect(withModifiers("\x1b[24~", CTRL_ALT)).toBe("\x1b[24;7~");
  });

  test("Escape and Tab", () => {
    expect(withModifiers("\x1b", ALT)).toBe("\x1b\x1b");
    expect(withModifiers("\t", ALT)).toBe("\x1b\t");
    expect(withModifiers("\t", CTRL)).toBe("\t");
  });

  test("unchanged", () => {
    expect(withModifiers("ls", CTRL_ALT)).toBe("ls");
    expect(withModifiers("\x1b[I", CTRL)).toBe("\x1b[I");
    expect(withModifiers("\x1b[200~ls\x1b[201~", CTRL)).toBe(
      "\x1b[200~ls\x1b[201~",
    );
  });
});
