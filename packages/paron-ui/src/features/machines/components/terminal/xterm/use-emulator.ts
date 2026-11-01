import {
  ClipboardAddon,
  type IClipboardProvider,
} from "@xterm/addon-clipboard";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { UnicodeGraphemesAddon } from "@xterm/addon-unicode-graphemes";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import { useEffect, useRef, useState } from "react";

import { useTheme } from "@/components/theme-provider";

import { isTouchDevice } from "../device/platform";
import { copyXtermStyles } from "./copy-xterm-styles";
import { onScreenKeysAddon, type OnScreenKeysAddon } from "./on-screen-keys";
import { terminalTheme } from "./theme";
import { repeatHeldBackspace } from "./touch-backspace";

export type Emulator = {
  xterm: Terminal;
  search: SearchAddon;
  onScreenKeys: OnScreenKeysAddon;
};

// The WebGL renderer floors a cell to whole device pixels (xtermjs/xterm.js#6015). Fira Code is
// 0.59999 em wide: at 15 px a glyph is 8.9999 px and its cell 8, too narrow; at 15.05 px the cell is
// 9, 18 and 27 at 1x, 2x and 3x. A phone keeps 13 px for the columns, and at 3x floors 0.4 of 23.4.
const FONT_SIZE = { desktop: 15.05, phone: 13 };
// xterm.js marks and counts this many matches and no more, because each mark has a cost. It is
// the default of the add-on, written here because the count that we show depends on it.
export const SEARCH_HIGHLIGHT_LIMIT = 1000;
export const HIDE_CURSOR = "\x1b[?25l";
export const SHOW_CURSOR = "\x1b[?25h";

/** A program on the machine may set the clipboard (OSC 52), never read it. */
const WRITE_ONLY_CLIPBOARD: IClipboardProvider = {
  readText: () => "",
  writeText: (_, text) => navigator.clipboard.writeText(text),
};

/**
 * One xterm.js terminal for the life of the page, drawn in `container`. It exists a moment after
 * the page: the font loads first.
 *
 * xterm.js may take Ghostty's terminal core (xtermjs/xterm.js#5686); if it does, the emulation
 * improves here with no change. A web build of libghostty itself is not mature enough to use yet.
 */
export function useEmulator() {
  const container = useRef<HTMLDivElement>(null);
  const [emulator, setEmulator] = useState<Emulator>();
  const { resolvedTheme } = useTheme();

  useEffect(() => {
    const element = container.current;
    if (!element) return;

    let cancelled = false;
    let xterm: Terminal | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let stopStyleCopy: (() => void) | undefined;
    let stopHeldBackspace: (() => void) | undefined;

    // Cells are measured when the terminal opens, so the font must be there first.
    const fontSize = isTouchDevice() ? FONT_SIZE.phone : FONT_SIZE.desktop;
    // The font of the container, which CSS sets.
    const { fontFamily } = getComputedStyle(element);
    void document.fonts.load(`${fontSize}px ${fontFamily}`).finally(() => {
      if (cancelled) return;

      xterm = new Terminal({
        allowProposedApi: true,
        fontFamily,
        fontSize,
        scrollback: 1000,
        minimumContrastRatio: 4.5,
        theme: terminalTheme(element),
      });
      const fitAddon = new FitAddon();
      const searchAddon = new SearchAddon({
        highlightLimit: SEARCH_HIGHLIGHT_LIMIT,
      });
      const onScreenKeys = onScreenKeysAddon();
      // First, so that no other add-on can listen on the `onData` that it replaces.
      xterm.loadAddon(onScreenKeys);
      xterm.loadAddon(fitAddon);
      xterm.loadAddon(searchAddon);
      xterm.loadAddon(new UnicodeGraphemesAddon());
      xterm.unicode.activeVersion = "15-graphemes";
      xterm.loadAddon(new ClipboardAddon(undefined, WRITE_ONLY_CLIPBOARD));
      xterm.loadAddon(
        new WebLinksAddon((_, uri) =>
          window.open(uri, "_blank", "noopener,noreferrer"),
        ),
      );

      stopStyleCopy = copyXtermStyles(element);
      xterm.open(element);
      xterm.write(HIDE_CURSOR);
      // The DOM renderer is the fallback when the browser has no WebGL, or loses its context.
      try {
        const webglAddon = new WebglAddon();
        webglAddon.onContextLoss(() => webglAddon.dispose());
        xterm.loadAddon(webglAddon);
      } catch {
        // No WebGL here: the DOM renderer draws.
      }

      fitAddon.fit();
      resizeObserver = new ResizeObserver(() => fitAddon.fit());
      resizeObserver.observe(element);
      stopHeldBackspace = repeatHeldBackspace(xterm);

      setEmulator({ xterm, search: searchAddon, onScreenKeys });
    });

    return () => {
      cancelled = true;
      resizeObserver?.disconnect();
      stopStyleCopy?.();
      stopHeldBackspace?.();
      setEmulator(undefined);
      xterm?.dispose();
    };
  }, [container]);

  useEffect(() => {
    if (emulator && container.current)
      applyTheme(emulator.xterm, container.current);
    // The colours are in CSS; the theme of the page says when they have changed.
  }, [emulator, container, resolvedTheme]);

  return { container, emulator };
}

// xterm.js is configured by assignment; React state holds the instance, so the assignment is here.
function applyTheme(xterm: Terminal, container: Element) {
  xterm.options.theme = terminalTheme(container);
}
