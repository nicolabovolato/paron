import type { IDisposable, Terminal } from "@xterm/xterm";
import { useCallback, useSyncExternalStore } from "react";

import {
  isModifier,
  isTerminalReport,
  keySequence,
  NO_MODIFIERS,
  withModifiers,
  type ModifierKey,
  type Modifiers,
  type SpecialKey,
} from "./key-encoding";

/**
 * The on-screen keys, as an add-on of the terminal: the keys that a phone keyboard does not have.
 * Ctrl and Alt send nothing. They apply to the next key that xterm.js reports, from the screen or
 * from a keyboard, and then go off. Each other key is typed into xterm.js, as on a keyboard.
 *
 * xterm.js cannot change what its `onData` reports. So this add-on replaces `onData` on the
 * terminal with one that reports the same input with Ctrl and Alt applied.
 */
export function onScreenKeysAddon() {
  let terminal: Terminal | undefined;
  let terminalData: IDisposable | undefined;
  let modifiers = NO_MODIFIERS;
  const dataListeners = new Set<(typed: string) => void>();
  const changeListeners = new Set<() => void>();
  const setModifiers = (next: Modifiers) => {
    modifiers = next;
    for (const notify of changeListeners) notify();
  };

  return {
    activate: (xterm: Terminal) => {
      terminal = xterm;
      terminalData = xterm.onData((data) => {
        let modified = data;

        if (!isTerminalReport(data)) {
          modified = withModifiers(data, modifiers);
          setModifiers(NO_MODIFIERS);
        }

        for (const listener of dataListeners) listener(modified);
      });

      Object.defineProperty(xterm, "onData", {
        configurable: true,
        value: ((listener: (typed: string) => void) => {
          dataListeners.add(listener);
          return { dispose: () => dataListeners.delete(listener) };
        }) satisfies Terminal["onData"],
      });
    },
    dispose: () => {
      terminalData?.dispose();
      if (terminal) Reflect.deleteProperty(terminal, "onData");
    },
    press: (key: SpecialKey | ModifierKey) => {
      if (isModifier(key))
        setModifiers({ ...modifiers, [key]: !modifiers[key] });
      else
        terminal?.input(
          keySequence(key, terminal.modes.applicationCursorKeysMode),
        );
    },
    get modifiers() {
      return modifiers;
    },
    onChange: (listener: () => void) => {
      changeListeners.add(listener);
      return () => changeListeners.delete(listener);
    },
  };
}

export type OnScreenKeysAddon = ReturnType<typeof onScreenKeysAddon>;

/** Which modifiers are on, for the on-screen keys to show. */
export function useModifiers(onScreenKeys: OnScreenKeysAddon | undefined) {
  return useSyncExternalStore(
    useCallback(
      (notify: () => void) => onScreenKeys?.onChange(notify) ?? (() => {}),
      [onScreenKeys],
    ),
    () => onScreenKeys?.modifiers ?? NO_MODIFIERS,
  );
}
