import {
  createContext,
  useContext,
  useEffect,
  useEffectEvent,
  useState,
  type ReactNode,
} from "react";
import { flushSync } from "react-dom";

type Theme = "dark" | "light" | "system";

// public/theme.js reads the same key and media query before the first paint.
const STORAGE_KEY = "paron-theme";
const SYSTEM_DARK = "(prefers-color-scheme: dark)";

const ThemeContext = createContext<
  | {
      theme: Theme;
      resolvedTheme: Exclude<Theme, "system">;
      setTheme: (theme: Theme) => void;
    }
  | undefined
>(undefined);

function storedTheme() {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    return "system";
  }
}

function resolveTheme(theme: Theme, systemDark: boolean) {
  if (theme !== "system") return theme;
  return systemDark ? "dark" : "light";
}

function applyTheme(resolvedTheme: Exclude<Theme, "system">) {
  const dark = resolvedTheme === "dark";
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.classList.toggle("light", !dark);
}

/** The browser cross-fades a snapshot of the page, so `change` must update the class before it returns. */
function withViewTransition(change: () => void) {
  if (
    !("startViewTransition" in document) ||
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  ) {
    change();
    return;
  }

  document.startViewTransition(change);
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<Theme>(storedTheme);
  const [systemDark, setSystemDark] = useState(
    () => window.matchMedia(SYSTEM_DARK).matches,
  );
  const resolvedTheme = resolveTheme(theme, systemDark);

  const followSystem = useEffectEvent((nextSystemDark: boolean) => {
    if (theme !== "system") {
      setSystemDark(nextSystemDark);
      return;
    }

    withViewTransition(() => {
      applyTheme(resolveTheme(theme, nextSystemDark));
      flushSync(() => setSystemDark(nextSystemDark));
    });
  });

  useEffect(() => {
    const system = window.matchMedia(SYSTEM_DARK);
    const follow = () => followSystem(system.matches);
    system.addEventListener("change", follow);
    return () => system.removeEventListener("change", follow);
  }, []);

  const setTheme = (nextTheme: Theme) => {
    try {
      localStorage.setItem(STORAGE_KEY, nextTheme);
    } catch {
      // Storage is blocked; the theme still changes for this session.
    }

    withViewTransition(() => {
      applyTheme(resolveTheme(nextTheme, systemDark));
      flushSync(() => setThemeState(nextTheme));
    });
  };

  return (
    <ThemeContext value={{ theme, resolvedTheme, setTheme }}>
      {children}
    </ThemeContext>
  );
}

// eslint-disable-next-line react-refresh/only-export-components
export function useTheme() {
  const context = useContext(ThemeContext);
  if (context === undefined)
    throw new Error("useTheme must be used within a ThemeProvider");

  return context;
}
