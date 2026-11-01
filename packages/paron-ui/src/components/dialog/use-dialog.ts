import {
  createContext,
  use,
  useEffect,
  useMemo,
  useRef,
  type ReactNode,
} from "react";

export type DialogApi = {
  /**
   * Resolves `true` when the dialog calls `close()`, `false` when dismissed.
   * The dialog closes when the caller of `useDialog()` unmounts.
   */
  open: (content: ReactNode) => Promise<boolean>;
  close: () => void;
};

export type DialogScope = {
  open: (content: ReactNode) => {
    completed: Promise<boolean>;
    dismiss: () => void;
  };
  close: () => void;
};

export const DialogContext = createContext<DialogScope | undefined>(undefined);

export function useDialog() {
  const scope = use(DialogContext);
  if (scope === undefined)
    throw new Error("useDialog must be used within a DialogProvider");

  const opened = useRef(new Set<() => void>());
  useEffect(() => {
    const openedDialogs = opened.current;
    return () => {
      for (const dismiss of openedDialogs) dismiss();
    };
  }, []);

  return useMemo(
    () =>
      ({
        open: (content) => {
          const { completed, dismiss } = scope.open(content);
          opened.current.add(dismiss);
          return completed.finally(() => opened.current.delete(dismiss));
        },
        close: scope.close,
      }) satisfies DialogApi,
    [scope],
  );
}
