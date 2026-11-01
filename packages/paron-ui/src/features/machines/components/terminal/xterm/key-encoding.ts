export type Modifiers = { ctrl: boolean; alt: boolean };
export type ModifierKey = keyof Modifiers;

export const NO_MODIFIERS: Modifiers = { ctrl: false, alt: false };

export function isModifier(key: string): key is ModifierKey {
  return key in NO_MODIFIERS;
}

// The encodings of the terminfo entry xterm-256color, which is the TERM that the server sends to
// the machine, and what xterm.js sends for these keys. A modifier m is 1 + 2 for Alt + 4 for Ctrl.
// cursor: `ESC [ A`, `ESC O A` in application cursor mode, `ESC [ 1 ; m A` with a modifier.
// ss3: `ESC O P`, `ESC [ 1 ; m P` with a modifier. tilde: `ESC [ 5 ~`, `ESC [ 5 ; m ~`.
type Sequence = { cursor: string } | { ss3: string } | { tilde: number };

const KEY_SEQUENCES = {
  ArrowUp: { cursor: "A" },
  ArrowDown: { cursor: "B" },
  ArrowRight: { cursor: "C" },
  ArrowLeft: { cursor: "D" },
  Home: { cursor: "H" },
  End: { cursor: "F" },
  PageUp: { tilde: 5 },
  PageDown: { tilde: 6 },
  F1: { ss3: "P" },
  F2: { ss3: "Q" },
  F3: { ss3: "R" },
  F4: { ss3: "S" },
  F5: { tilde: 15 },
  F6: { tilde: 17 },
  F7: { tilde: 18 },
  F8: { tilde: 19 },
  F9: { tilde: 20 },
  F10: { tilde: 21 },
  F11: { tilde: 23 },
  F12: { tilde: 24 },
} satisfies Record<string, Sequence>;

export type SpecialKey = "Escape" | "Tab" | keyof typeof KEY_SEQUENCES;

/** What a keyboard sends for a special key, with no modifier. */
export function keySequence(key: SpecialKey, applicationCursor: boolean) {
  if (key === "Escape") return "\x1b";
  if (key === "Tab") return "\t";
  return sequenceWithNoModifier(KEY_SEQUENCES[key], applicationCursor);
}

function sequenceWithNoModifier(
  sequence: Sequence,
  applicationCursor: boolean,
) {
  if ("tilde" in sequence) return `\x1b[${sequence.tilde}~`;
  if ("ss3" in sequence) return `\x1bO${sequence.ss3}`;
  return applicationCursor
    ? `\x1bO${sequence.cursor}`
    : `\x1b[${sequence.cursor}`;
}

// The keys of the table by what they send with no modifier, in each cursor mode.
const KEYS_BY_SEQUENCE_WITH_NO_MODIFIER = new Map(
  Object.values<Sequence>(KEY_SEQUENCES).flatMap((sequence) => [
    [sequenceWithNoModifier(sequence, false), sequence] as const,
    [sequenceWithNoModifier(sequence, true), sequence] as const,
  ]),
);

/**
 * What the terminal sends by itself and not for a key: a mouse, focus or status report, and a
 * paste in bracketed paste mode. Each of these starts with ESC and is not the sequence of a key.
 */
export function isTerminalReport(data: string) {
  return (
    data.length > 1 &&
    data.startsWith("\x1b") &&
    !KEYS_BY_SEQUENCE_WITH_NO_MODIFIER.has(data)
  );
}

/**
 * One key that a keyboard sent, a character or the sequence of a special key, as the same key with
 * Ctrl and Alt. Other text, for example a paste, does not change.
 */
export function withModifiers(typed: string, { ctrl, alt }: Modifiers) {
  if (!ctrl && !alt) return typed;

  const specialKey = KEYS_BY_SEQUENCE_WITH_NO_MODIFIER.get(typed);
  if (specialKey) {
    const modifier = 1 + (alt ? 2 : 0) + (ctrl ? 4 : 0);
    if ("tilde" in specialKey) return `\x1b[${specialKey.tilde};${modifier}~`;
    const finalCharacter =
      "ss3" in specialKey ? specialKey.ss3 : specialKey.cursor;
    return `\x1b[1;${modifier}${finalCharacter}`;
  }

  if (typed.length !== 1) return typed;
  let bytes = typed;
  if (ctrl) {
    const code = typed.toUpperCase().charCodeAt(0);
    // Ctrl clears bits 6 and 7: @ A–Z [ \ ] ^ _ become 0x00–0x1F, as on a real keyboard.
    if (code >= 0x40 && code <= 0x5f) bytes = String.fromCharCode(code & 0x1f);
    else if (typed === " ") bytes = "\x00";
    else if (typed === "?") bytes = "\x7f";
  }

  return alt ? `\x1b${bytes}` : bytes;
}
