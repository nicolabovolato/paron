import { SHORTCUTS_USE_COMMAND_KEY } from "./platform";

const SHORTCUT_LETTERS = { copy: "C", paste: "V", search: "F" } as const;

type Shortcut = keyof typeof SHORTCUT_LETTERS;

export function shortcutLabel(shortcut: Shortcut) {
  const letter = SHORTCUT_LETTERS[shortcut];
  return SHORTCUTS_USE_COMMAND_KEY ? `⌘${letter}` : `Ctrl+Shift+${letter}`;
}

export function isShortcut(
  event: Pick<
    KeyboardEvent,
    "key" | "code" | "metaKey" | "ctrlKey" | "shiftKey"
  >,
  shortcut: Shortcut,
) {
  const modifiers = SHORTCUTS_USE_COMMAND_KEY
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && event.shiftKey;

  const pressed = /^[a-z]$/i.test(event.key)
    ? event.key.toUpperCase()
    : event.code.replace(/^Key/, "");

  return modifiers && pressed === SHORTCUT_LETTERS[shortcut];
}
