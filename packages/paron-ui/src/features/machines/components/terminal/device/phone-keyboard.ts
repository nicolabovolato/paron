import type { Terminal } from "@xterm/xterm";
import {
  useEffect,
  useState,
  useSyncExternalStore,
  type MouseEvent,
  type PointerEvent,
} from "react";

import {
  isTouchDevice,
  subscribeToTouchDevice,
  ZOOMS_INTO_SMALL_FIELDS,
} from "./platform";

/**
 * iOS puts its keyboard over the page and does not shrink it, so the terminal must leave room for
 * it. iOS reports the keyboard after its animation, so the room changes then.
 */
export function useKeyboardHeight() {
  const height = useSyncExternalStore(
    subscribeToKeyboardHeight,
    keyboardHeight,
  );
  const keyboardOpen = height > 0;

  useEffect(() => {
    if (!keyboardOpen) return;

    const onTouchMove = (event: TouchEvent) => {
      if (event.touches.length !== 1) return;
      if (scrollY > 0 || (visualViewport?.offsetTop ?? 0) > 0) return;
      for (
        let element = event.target instanceof Element ? event.target : null;
        element;
        element = element.parentElement
      ) {
        const scrollable =
          element.scrollHeight > element.clientHeight &&
          /auto|scroll/.test(getComputedStyle(element).overflowY);
        if (scrollable) return;
      }
      event.preventDefault();
    };
    document.addEventListener("touchmove", onTouchMove, { passive: false });
    return () => document.removeEventListener("touchmove", onTouchMove);
  }, [keyboardOpen]);

  return height;
}

function subscribeToKeyboardHeight(notify: () => void) {
  visualViewport?.addEventListener("resize", notify);
  const unsubscribeFromTouchDevice = subscribeToTouchDevice(notify);
  return () => {
    visualViewport?.removeEventListener("resize", notify);
    unsubscribeFromTouchDevice();
  };
}

function keyboardHeight() {
  if (!visualViewport || !isTouchDevice() || visualViewport.scale > 1) return 0;
  return Math.max(0, Math.round(innerHeight - visualViewport.height));
}

/**
 * iOS zooms into a focused text field under 16px, as the search field is, unless the page sets
 * maximum-scale=1; since iOS 10 it still lets the user pinch zoom. Elsewhere maximum-scale=1 blocks
 * pinch zoom, so only on iOS, and only while this page is open.
 *
 * Android Chrome puts its keyboard over the page, and over the key bar, unless the page sets
 * interactive-widget=resizes-content. Only this page: elsewhere it shrinks the page under a dialog
 * that does not scroll.
 */
export function useTerminalViewportMeta() {
  useEffect(() => {
    const settings = `interactive-widget=resizes-content${ZOOMS_INTO_SMALL_FIELDS ? ", maximum-scale=1" : ""}`;
    const viewport = document.querySelector<HTMLMetaElement>(
      'meta[name="viewport"]',
    );
    if (!viewport) {
      const added = document.createElement("meta");
      added.name = "viewport";
      added.content = settings;
      document.head.append(added);
      return () => added.remove();
    }

    const original = viewport.content;
    viewport.content = `${original}, ${settings}`;
    return () => {
      viewport.content = original;
    };
  }, []);
}

/**
 * The handlers of a button that must not take the focus from a text field: a phone closes its
 * keyboard when the field loses it. Cancelling `pointerdown` keeps the focus, but WebKit then fires
 * no `click`, so a pointer presses on `pointerup`; `click` still serves the keyboard, which has no
 * pointer and fires it with `detail` 0.
 */
export function pressKeepingFocus(onClick: () => void) {
  return {
    onPointerDown: (event: PointerEvent) => event.preventDefault(),
    onMouseDown: (event: MouseEvent) => event.preventDefault(),
    onPointerUp: (event: PointerEvent) => {
      if (event.button === 0) onClick();
    },
    onClick: (event: MouseEvent) => {
      if (event.detail === 0) onClick();
    },
  };
}

export function usePhoneKeyboardSwitch(xterm: Terminal | undefined) {
  const [enabled, setEnabled] = useState(true);

  useEffect(() => {
    const textarea = xterm?.textarea;
    if (!textarea || enabled) return;

    const removeFocus = () => textarea.blur();
    textarea.addEventListener("focus", removeFocus);
    return () => textarea.removeEventListener("focus", removeFocus);
  }, [xterm, enabled]);

  return {
    enabled,
    toggle: () => {
      const textarea = xterm?.textarea;
      if (!textarea) return;
      if (enabled) {
        textarea.setAttribute("inputmode", "none");
        textarea.blur();
      } else {
        textarea.removeAttribute("inputmode");
        textarea.focus();
      }
      setEnabled(!enabled);
    },
  };
}
