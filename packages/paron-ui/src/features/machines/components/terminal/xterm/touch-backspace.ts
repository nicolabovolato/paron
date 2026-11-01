import type { Terminal } from "@xterm/xterm";

import { isTouchDevice } from "../device/platform";

const INVISIBLE_CHARACTER = "​";

/**
 * A held Backspace on a phone keyboard. iOS repeats it only while the field it edits has text before
 * the caret, and xterm.js keeps its hidden textarea empty; so the textarea always holds one
 * invisible character. iOS sends a real Backspace keydown, which xterm.js would handle and cancel,
 * and a cancelled keydown stops the repeat; so xterm.js lets it pass (see `isBackspaceOnTouchDevice`), and each
 * repeat arrives here as a `beforeinput`: a DEL, or Ctrl+W once iOS deletes whole words.
 * Android keyboards send keydown 229 instead, and xterm.js already sends a DEL when the textarea
 * gets shorter; there the invisible character alone is what makes the repeat work.
 */
export function repeatHeldBackspace(xterm: Terminal) {
  const textarea = xterm.textarea;
  if (!textarea || !isTouchDevice()) return () => {};

  let lastKeyWasBackspace = false;
  const fillEmptyTextarea = () => {
    if (textarea.value !== "") return;
    textarea.value = INVISIBLE_CHARACTER;
    textarea.setSelectionRange(
      INVISIBLE_CHARACTER.length,
      INVISIBLE_CHARACTER.length,
    );
  };

  const onKeyDown = (event: KeyboardEvent) => {
    lastKeyWasBackspace = event.key === "Backspace";
    if (lastKeyWasBackspace) fillEmptyTextarea();
  };
  const onBeforeInput = (event: InputEvent) => {
    if (!lastKeyWasBackspace || event.isComposing) return;
    if (event.inputType === "deleteContentBackward") xterm.input("\x7f");
    else if (event.inputType === "deleteWordBackward") xterm.input("\x17");
  };
  const onInput = () => setTimeout(fillEmptyTextarea, 0);

  textarea.addEventListener("focus", fillEmptyTextarea);
  textarea.addEventListener("keydown", onKeyDown, { capture: true });
  textarea.addEventListener("beforeinput", onBeforeInput);
  textarea.addEventListener("input", onInput);
  fillEmptyTextarea();

  return () => {
    textarea.removeEventListener("focus", fillEmptyTextarea);
    textarea.removeEventListener("keydown", onKeyDown, { capture: true });
    textarea.removeEventListener("beforeinput", onBeforeInput);
    textarea.removeEventListener("input", onInput);
  };
}

/** On a touch device, xterm.js must not handle Backspace; see `repeatHeldBackspace`. */
export function isBackspaceOnTouchDevice(event: KeyboardEvent) {
  return event.key === "Backspace" && isTouchDevice();
}
