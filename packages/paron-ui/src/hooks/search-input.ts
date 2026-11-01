import { useEffect, useEffectEvent, useState } from "react";
import type { z } from "zod";

const SEARCH_DELAY_MS = 200;

export function useSearchInput(
  search: string | undefined,
  onSearchChange: (search: string | undefined) => void,
  searchSchema: z.ZodType<string | undefined>,
) {
  const [inputText, setInputText] = useState(search ?? "");
  const typedSearch = inputText.trim() || undefined;
  const checkedSearch = searchSchema.safeParse(typedSearch);

  const [previousSearch, setPreviousSearch] = useState(search);
  if (search !== previousSearch) {
    setPreviousSearch(search);
    if (search !== typedSearch) setInputText(search ?? "");
  }

  const sendTypedSearch = useEffectEvent(
    (delayedSearch: string | undefined) => {
      if (delayedSearch !== search) onSearchChange(delayedSearch);
    },
  );

  useEffect(() => {
    if (!checkedSearch.success) return;
    const timer = setTimeout(
      () => sendTypedSearch(typedSearch),
      SEARCH_DELAY_MS,
    );
    return () => clearTimeout(timer);
  }, [typedSearch, checkedSearch.success]);

  const clear = () => {
    setInputText("");
    onSearchChange(undefined);
  };

  return {
    inputText,
    changeInputText: setInputText,
    clear,
    pending: checkedSearch.success && typedSearch !== search,
    errors: checkedSearch.error?.issues ?? [],
  };
}
