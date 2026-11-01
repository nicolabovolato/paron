import { useOptimistic, useTransition } from "react";

export function useOptimisticSearch<Search>(
  search: Search,
  navigateToSearch: (search: Search) => Promise<void>,
) {
  const [pending, startTransition] = useTransition();
  const [shownSearch, setShownSearch] = useOptimistic(search);

  const changeSearch = (nextSearch: Search) =>
    startTransition(async () => {
      setShownSearch(nextSearch);
      await navigateToSearch(nextSearch);
    });

  return { shownSearch, changeSearch, pending };
}
