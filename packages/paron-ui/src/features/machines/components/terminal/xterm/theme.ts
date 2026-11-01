import type { ITheme } from "@xterm/xterm";

function cssVars(container: Element) {
  const variables = getComputedStyle(container);
  return (name: string) => {
    const value = variables.getPropertyValue(name).trim();
    if (!value) throw new Error(`terminal.module.css has no ${name}`);
    return value;
  };
}

export function terminalTheme(container: Element) {
  const cssVar = cssVars(container);
  return {
    background: cssVar("--terminal-background"),
    foreground: cssVar("--terminal-foreground"),
    cursor: cssVar("--terminal-cursor"),
    cursorAccent: cssVar("--terminal-cursor-accent"),
    selectionBackground: cssVar("--terminal-selection-background"),
    selectionInactiveBackground: cssVar(
      "--terminal-selection-inactive-background",
    ),
    black: cssVar("--terminal-black"),
    red: cssVar("--terminal-red"),
    green: cssVar("--terminal-green"),
    yellow: cssVar("--terminal-yellow"),
    blue: cssVar("--terminal-blue"),
    magenta: cssVar("--terminal-magenta"),
    cyan: cssVar("--terminal-cyan"),
    white: cssVar("--terminal-white"),
    brightBlack: cssVar("--terminal-bright-black"),
    brightRed: cssVar("--terminal-bright-red"),
    brightGreen: cssVar("--terminal-bright-green"),
    brightYellow: cssVar("--terminal-bright-yellow"),
    brightBlue: cssVar("--terminal-bright-blue"),
    brightMagenta: cssVar("--terminal-bright-magenta"),
    brightCyan: cssVar("--terminal-bright-cyan"),
    brightWhite: cssVar("--terminal-bright-white"),
  } satisfies ITheme;
}

export function searchDecorations(container: Element) {
  const cssVar = cssVars(container);
  return {
    matchBackground: cssVar("--terminal-match"),
    matchOverviewRuler: cssVar("--terminal-match-ruler"),
    activeMatchBackground: cssVar("--terminal-active-match"),
    activeMatchColorOverviewRuler: cssVar("--terminal-active-match-ruler"),
  };
}
