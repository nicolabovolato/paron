import { useSyncExternalStore } from "react";

let narrowScreen: MediaQueryList | undefined;

function narrowScreenQuery() {
  narrowScreen ??= matchMedia(
    `(width < ${getComputedStyle(document.documentElement).getPropertyValue("--breakpoint-md")})`,
  );
  return narrowScreen;
}

function isNarrowScreen() {
  return narrowScreenQuery().matches;
}

function subscribeToNarrowScreen(notify: () => void) {
  const query = narrowScreenQuery();
  query.addEventListener("change", notify);
  return () => query.removeEventListener("change", notify);
}

export function useNarrowScreen() {
  return useSyncExternalStore(subscribeToNarrowScreen, isNarrowScreen);
}
