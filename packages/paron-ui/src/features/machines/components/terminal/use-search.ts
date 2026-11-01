import { useEffect, useRef, useState } from "react";

import { searchDecorations } from "./xterm/theme";
import { SEARCH_HIGHLIGHT_LIMIT, type Emulator } from "./xterm/use-emulator";

type SearchDirection = "next" | "previous";

export function useSearch(emulator: Emulator | undefined) {
  const field = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<{
    position: number | undefined;
    total: number;
    limited: boolean;
  }>();

  useEffect(() => {
    if (!emulator) return;

    const subscription = emulator.search.onDidChangeResults(
      ({ resultIndex, resultCount }) =>
        setMatches({
          position: resultIndex === -1 ? undefined : resultIndex + 1,
          total: resultCount,
          limited: resultCount >= SEARCH_HIGHLIGHT_LIMIT,
        }),
    );
    return () => subscription.dispose();
  }, [emulator]);

  const clear = () => {
    emulator?.search.clearDecorations();
    setMatches(undefined);
  };
  const find = (direction: SearchDirection, searchedText: string) => {
    const terminalElement = emulator?.xterm.element;
    if (!emulator || !terminalElement) return;
    if (!searchedText) {
      clear();
      return;
    }
    const options = { decorations: searchDecorations(terminalElement) };
    if (direction === "next") emulator.search.findNext(searchedText, options);
    else emulator.search.findPrevious(searchedText, options);
  };
  const show = () => {
    setOpen(true);
    find("next", query);
  };
  const onClose = () => {
    setOpen(false);
    clear();
    emulator?.xterm.focus();
  };

  return {
    open,
    query,
    matches,
    field,
    onQueryChange: (typedQuery: string) => {
      setQuery(typedQuery);
      find("next", typedQuery);
    },
    onFind: (direction: SearchDirection) => find(direction, query),
    onClose,
    toggle: () => {
      if (open) onClose();
      else show();
    },
    openAndFocus: () => {
      if (!open) {
        show();
        return;
      }
      field.current?.focus();
      field.current?.select();
    },
  };
}
