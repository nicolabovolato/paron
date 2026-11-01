import { useState, useSyncExternalStore } from "react";

function subscribeToFullScreen(notify: () => void) {
  document.addEventListener("fullscreenchange", notify);
  return () => document.removeEventListener("fullscreenchange", notify);
}

export function useFullScreen() {
  const nativeFullScreen = useSyncExternalStore(
    subscribeToFullScreen,
    () => document.fullscreenElement !== null,
  );
  const [coveringPage, setCoveringPage] = useState(false); // iPhone Safari has no Fullscreen API for anything but video; there the terminal covers the page.

  return {
    active: nativeFullScreen || coveringPage,
    toggle: async () => {
      if (nativeFullScreen) await document.exitFullscreen();
      else if (coveringPage) setCoveringPage(false);
      else
        try {
          await document.documentElement.requestFullscreen();
        } catch {
          setCoveringPage(true);
        }
    },
  };
}
