import { useEffect, useEffectEvent, useState } from "react";

import type { Protocol } from "@/generated/api/types.gen";

import { usePhoneKeyboardSwitch } from "./device/phone-keyboard";
import { isShortcut } from "./device/shortcuts";
import { useSearch } from "./use-search";
import { useSession } from "./use-session";
import type { ModifierKey, SpecialKey } from "./xterm/key-encoding";
import { useModifiers } from "./xterm/on-screen-keys";
import { isBackspaceOnTouchDevice } from "./xterm/touch-backspace";
import { handleTouchGestures } from "./xterm/touch-gestures";
import { useEmulator } from "./xterm/use-emulator";

export function useTerminal(machineId: string, protocol: Protocol) {
  const { container, emulator } = useEmulator();
  const xterm = emulator?.xterm;
  const modifiers = useModifiers(emulator?.onScreenKeys);
  const session = useSession(emulator, machineId, protocol);
  const search = useSearch(emulator);
  const phoneKeyboard = usePhoneKeyboardSwitch(xterm);

  const openSearch = useEffectEvent(search.openAndFocus);
  useEffect(() => {
    if (!xterm) return;

    xterm.attachCustomKeyEventHandler((event) => {
      if (isBackspaceOnTouchDevice(event)) return false;
      if (event.type !== "keydown") return true;

      if (isShortcut(event, "search")) {
        event.preventDefault();
        openSearch();
        return false;
      }
      if (isShortcut(event, "copy") && xterm.hasSelection()) {
        event.preventDefault();
        void navigator.clipboard.writeText(xterm.getSelection());
        return false;
      }
      if (isShortcut(event, "paste")) return false;

      return true;
    });
  }, [xterm]);

  const [hasSelection, setHasSelection] = useState(false);
  useEffect(() => {
    if (!xterm) return;

    const subscription = xterm.onSelectionChange(() =>
      setHasSelection(xterm.hasSelection()),
    );
    return () => subscription.dispose();
  }, [xterm]);

  const [touchMenuOpen, setTouchMenuOpen] = useState(false);
  const [touchMenuAnchor, setTouchMenuAnchor] = useState(() => new DOMRect());
  useEffect(() => {
    if (!xterm || !container.current) return;

    return handleTouchGestures(xterm, container.current, (menuAnchor) => {
      setTouchMenuAnchor(menuAnchor);
      setTouchMenuOpen(true);
    });
  }, [xterm, container]);

  const connected = session.status.kind === "connected";

  return {
    container,
    session,
    actions: {
      focus: () => xterm?.focus(),
      inputElement: () => xterm?.textarea,
      copy: hasSelection
        ? () => {
            if (xterm) void navigator.clipboard.writeText(xterm.getSelection());
          }
        : undefined,
      paste: connected
        ? () => {
            if (xterm)
              void navigator.clipboard
                .readText()
                .then((text) => xterm.paste(text));
          }
        : undefined,
      selectAll: () => xterm?.selectAll(),
      clearSelection: () => xterm?.clearSelection(),
      clear: connected ? () => xterm?.clear() : undefined,
    },
    searchBar: search,
    touchContextMenu: {
      open: touchMenuOpen,
      anchor: touchMenuAnchor,
      onClose: () => setTouchMenuOpen(false),
    },
    keys: {
      modifiers,
      onClick: (key: SpecialKey | ModifierKey) =>
        emulator?.onScreenKeys.press(key),
    },
    phoneKeyboard,
  };
}
