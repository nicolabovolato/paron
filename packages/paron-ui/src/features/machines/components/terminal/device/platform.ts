import { useSyncExternalStore } from "react";

const MACOS = /Macintosh/.test(navigator.userAgent);

const IOS =
  /iPad|iPhone|iPod/.test(navigator.userAgent) ||
  (MACOS && navigator.maxTouchPoints > 1);

/** macOS keeps Ctrl for the shell and has its shortcuts on Cmd. */
export const SHORTCUTS_USE_COMMAND_KEY = MACOS || IOS;

/** iOS zooms into a focused text field whose text is smaller than 16px. */
export const ZOOMS_INTO_SMALL_FIELDS = IOS;

const TOUCH_DEVICE = "(pointer: coarse)";

export function isTouchDevice() {
  return matchMedia(TOUCH_DEVICE).matches;
}

export function subscribeToTouchDevice(notify: () => void) {
  const query = matchMedia(TOUCH_DEVICE);
  query.addEventListener("change", notify);
  return () => query.removeEventListener("change", notify);
}

export function useTouchDevice() {
  return useSyncExternalStore(subscribeToTouchDevice, isTouchDevice);
}
